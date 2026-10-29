//! S1 with the fake Google server: the shipped `google_calendar` task pulling Google Calendar
//! events into `event` notes (google-sync D7, D8, D9, D10, D12, D16, D18, D19).
//!
//! The server runs on a test clock, so `vault::today()` is fixed: 2026-03-01, with the occurrence
//! window from 2026-01-30 to 2027-03-01.

use std::collections::BTreeMap;
use std::ffi::OsStr;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::fake_google::{DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN};
use testkit::{FakeGoogle, TestServer};

const START: &str = "2026-03-01T08:00:00Z";

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

struct Api {
    server: TestServer,
    client: Client,
    google: FakeGoogle,
}

impl Api {
    /// A server on the test clock whose `google_calendar` task reaches the fake, with the three
    /// secrets set, and no settings note yet.
    fn bare() -> Api {
        let google = FakeGoogle::start();
        let server =
            TestServer::start_with_args(|_| {}, &[OsStr::new("--test-clock"), OsStr::new(START)]);
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder().default_headers(headers).build().unwrap();
        let api = Api {
            server,
            client,
            google,
        };
        for (name, value) in [
            ("google_client_id", DEFAULT_CLIENT_ID),
            ("google_client_secret", DEFAULT_CLIENT_SECRET),
            ("google_refresh_token", DEFAULT_REFRESH_TOKEN),
        ] {
            expect(
                api.req("PUT", &format!("/api/secrets/{name}"))
                    .json(&json!({ "value": value }))
                    .send()
                    .unwrap(),
                200,
            );
        }
        // The shipped task, with the fake's host allowed as the spec's tests do.
        let task =
            std::fs::read_to_string(api.server.vault_path().join("tasks/google_calendar.wcl"))
                .unwrap();
        let task = task.replace(
            "allow_domains = [",
            &format!("allow_domains = [\"{}\", ", api.google.host()),
        );
        expect(
            api.req("PUT", "/api/tasks/google_calendar")
                .body(task)
                .send()
                .unwrap(),
            200,
        );
        api
    }

    /// [`Api::bare`] with the `google_settings` secret pointing at the fake.
    fn start() -> Api {
        let api = Api::bare();
        api.put_settings(json!({}));
        api
    }

    /// Store `settings`, with the fake's endpoints, as the `google_settings` secret.
    fn put_settings(&self, mut settings: Value) {
        settings["endpoints"] = self.google.endpoints();
        self.put_secret("google_settings", &settings.to_string());
    }

    fn put_secret(&self, name: &str, value: &str) {
        expect(
            self.req("PUT", &format!("/api/secrets/{name}"))
                .json(&json!({ "value": value }))
                .send()
                .unwrap(),
            200,
        );
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
    }

    fn create_note(&self, type_name: &str, note: Value) -> String {
        let made = expect(
            self.req("POST", &format!("/api/notes?type={type_name}"))
                .json(&json!({ "note": note }))
                .send()
                .unwrap(),
            201,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notes/{id}")).send().unwrap(),
            200,
        )
    }

    /// Run the task now and return the run.
    fn sync(&self) -> Value {
        let run = expect(
            self.req("POST", "/api/tasks/google_calendar/run?wait=true")
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run}");
        run
    }

    fn query(&self, text: &str) -> Vec<Value> {
        let mut rows = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let mut body = json!({ "query": text, "page_size": 1000 });
            if let Some(c) = cursor.as_str() {
                body["cursor"] = json!(c);
            }
            let page = expect(
                self.req("POST", "/api/query").json(&body).send().unwrap(),
                200,
            );
            rows.extend(page["rows"].as_array().unwrap().iter().cloned());
            cursor = page["next"].clone();
            if cursor.is_null() {
                return rows;
            }
        }
    }

    /// Every event note that is not in the trash, by `google_ref`. Fails on two notes with one.
    fn events(&self) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        for row in self.query("from event include archived") {
            let r = row["fields"]["google_ref"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            assert!(out.insert(r.clone(), row).is_none(), "two notes for {r}");
        }
        out
    }

    fn event_id(&self, google_ref: &str) -> String {
        self.events()[google_ref]["id"].as_str().unwrap().to_owned()
    }

    /// The Calendar task's state: the JSON object in the `google_calendar_state` secret.
    fn state(&self) -> Value {
        self.server.secret_json("google_calendar_state")
    }

    fn trashed(&self) -> Vec<String> {
        let trash = expect(self.req("GET", "/api/trash").send().unwrap(), 200);
        trash["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Change fields of note `id` as the user would.
    fn edit(&self, id: &str, changes: Value) {
        let mut note = self.note(id);
        for (k, v) in changes.as_object().unwrap() {
            note["note"]["fields"][k] = v.clone();
        }
        expect(
            self.req("PUT", &format!("/api/notes/{id}"))
                .json(&json!({ "note": note["note"], "base_version": note["version"] }))
                .send()
                .unwrap(),
            200,
        );
    }

    fn advance(&self, by: &str) {
        expect(
            self.req("POST", "/api/test/clock")
                .json(&json!({ "advance": by }))
                .send()
                .unwrap(),
            200,
        );
    }

    fn notifications(&self) -> Vec<Value> {
        expect(self.req("GET", "/api/notifications").send().unwrap(), 200)["items"]
            .as_array()
            .unwrap()
            .clone()
    }
}

/// The members of note `note`'s block of `kind` labelled `name`.
fn block(note: &Value, kind: &str, name: &str) -> Value {
    note["note"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == kind && b["labels"] == json!([name]))
        .cloned()
        .unwrap_or(Value::Null)
}

/// The paragraphs of note `note`'s content field `name`, one per line.
fn text(note: &Value, name: &str) -> String {
    block(note, "content", name)["blocks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["fields"]["text"].as_str().unwrap_or("").to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Days since 1970-01-01 of `YYYY-MM-DD`.
fn day(date: &str) -> i64 {
    let n = |r: std::ops::Range<usize>| date[r].parse::<i64>().unwrap();
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

#[test]
fn a_new_vault_ships_the_calendar_task_paused_and_a_run_without_settings_does_nothing() {
    let api = Api::bare();
    let task = expect(
        api.req("GET", "/api/tasks/google_calendar").send().unwrap(),
        200,
    );
    assert_eq!(task["paused"], true, "{task}");
    let wcl =
        std::fs::read_to_string(api.server.vault_path().join("tasks/google_calendar.wcl")).unwrap();
    for want in [
        "schedule      = \"*/5 * * * *\"",
        "script        = \"google_calendar\"",
        "timeout       = \"PT5M\"",
        "\"oauth2.googleapis.com\", \"gmail.googleapis.com\", \"www.googleapis.com\"",
        "secrets       = [\"google_client_id\", \"google_client_secret\", \"google_refresh_token\", \"google_settings\", \"google_calendar_state\"]",
    ] {
        assert!(wcl.contains(want), "{want}: {wcl}");
    }
    assert!(
        api.server
            .vault_path()
            .join("scripts/google_calendar.ws")
            .exists()
    );

    // Without a refresh token Google is not connected, and the run ends doing nothing.
    expect(
        api.req("DELETE", "/api/secrets/google_refresh_token")
            .send()
            .unwrap(),
        200,
    );
    let run = api.sync();
    assert!(
        run["log"]
            .as_str()
            .unwrap()
            .contains("Google is not connected"),
        "{run}"
    );
    assert!(api.state().is_null());
    assert!(api.google.requests().is_empty());
}

#[test]
fn the_first_run_imports_single_events_all_day_as_dates_and_timed_with_offsets() {
    let api = Api::start();
    let ann = api.create_note(
        "person",
        json!({ "fields": { "title": "Ann", "emails": [{ "address": "Ann@Example.com" }] } }),
    );
    api.google.put_event(
        "primary",
        json!({
            "id": "holiday",
            "summary": "Holiday",
            "start": { "date": "2026-03-10" },
            "end": { "date": "2026-03-12" },
        }),
    );
    let standup = api.google.put_event(
        "primary",
        json!({
            "id": "standup",
            "summary": "Standup",
            "location": "Room 1",
            "description": "Agenda:\r\n\r\n- status  \n- blockers",
            "start": { "dateTime": "2026-03-02T09:30:00+11:00", "timeZone": "Australia/Melbourne" },
            "end": { "dateTime": "2026-03-02T10:00:00+11:00", "timeZone": "Australia/Melbourne" },
            "attendees": [{ "email": "ann@example.com" }, { "email": "bob@example.com" }],
            "organizer": { "email": "me@example.com" },
            "status": "tentative",
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
        }),
    );

    let run = api.sync();
    assert!(run["log"].as_str().unwrap().contains("2 created"), "{run}");

    // One token refresh, a form POST, then one full listing.
    let refreshes = api.google.requests_to("/token");
    assert_eq!(refreshes.len(), 1);
    assert_eq!(
        refreshes[0].form("grant_type").as_deref(),
        Some("refresh_token")
    );
    assert_eq!(
        refreshes[0].form("refresh_token").as_deref(),
        Some(DEFAULT_REFRESH_TOKEN)
    );
    let lists = api
        .google
        .requests_to("/calendar/v3/calendars/primary/events");
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert_eq!(lists[0].query("syncToken"), None);
    assert_eq!(lists[0].query("singleEvents").as_deref(), Some("false"));

    let events = api.events();
    assert_eq!(events.len(), 2, "{events:?}");
    let holiday = api.note(events["holiday"]["id"].as_str().unwrap());
    let f = &holiday["note"]["fields"];
    assert_eq!(f["title"], "Holiday");
    // Google's all-day end is the day after; the vault's range ends on the last day.
    assert_eq!(
        f["when"],
        json!({ "start": "2026-03-10", "end": "2026-03-11" })
    );
    assert_eq!(f["calendar"], "primary");
    assert_eq!(f["status"], "confirmed");

    let note = api.note(events["standup"]["id"].as_str().unwrap());
    let f = &note["note"]["fields"];
    assert_eq!(
        f["when"],
        json!({ "start": "2026-03-02T09:30:00+11:00", "end": "2026-03-02T10:00:00+11:00" })
    );
    assert_eq!(f["location"], "Room 1");
    assert_eq!(f["status"], "tentative");
    assert_eq!(f["organiser"], "me@example.com");
    assert_eq!(f["meet_url"], "https://meet.google.com/abc-defg-hij");
    assert_eq!(
        f["attendees_raw"],
        json!(["ann@example.com", "bob@example.com"])
    );
    assert_eq!(f["attendees"], json!([ann]), "only known people are linked");
    assert_eq!(f["google_ref"], "standup");
    assert_eq!(f["etag"], standup["etag"]);
    assert!(f.get("series").is_none(), "{f}");
    assert_eq!(text(&note, "description"), "Agenda:\n- status\n- blockers");
    assert_eq!(
        block(&note, "dict", "snapshot")["fields"],
        json!({
            "title": "Standup",
            "start": "2026-03-02T09:30:00+11:00",
            "end": "2026-03-02T10:00:00+11:00",
            "location": "Room 1",
            // As the HTML Google takes, which reads back as itself (rich-import H4).
            "description": "<p>Agenda:</p><p>- status<br>- blockers</p>",
            "attendees": "ann@example.com\nbob@example.com",
            "organiser": "me@example.com",
            "status": "tentative",
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "converter": "2",
        })
    );

    let f = api.state();
    assert_eq!(f["window_end"], "2027-03-01");
    assert_eq!(f["last_run"], START);
    assert!(f["last_error"].is_null(), "{f}");
    let tokens = &f["sync_tokens"];
    assert!(
        tokens["primary"].as_str().is_some_and(|t| !t.is_empty()),
        "{tokens}"
    );

    // Nothing changed in Google: the next run writes no note.
    let versions = note["version"].clone();
    let run = api.sync();
    assert!(
        run["log"]
            .as_str()
            .unwrap()
            .contains("0 created, 0 updated, 0 trashed"),
        "{run}"
    );
    assert_eq!(
        api.note(events["standup"]["id"].as_str().unwrap())["version"],
        versions
    );
}

#[test]
fn a_series_becomes_one_note_per_occurrence_in_the_window_and_follows_changes_and_cancels() {
    let api = Api::start();
    // Weekly on Mondays, all day, from before the window starts and with no end.
    api.google.put_event(
        "primary",
        json!({
            "id": "gym",
            "summary": "Gym",
            "start": { "date": "2026-01-05" },
            "end": { "date": "2026-01-06" },
            "recurrence": ["RRULE:FREQ=WEEKLY"],
        }),
    );
    // A timed weekly series of three.
    api.google.put_event(
        "primary",
        json!({
            "id": "review",
            "summary": "Review",
            "start": { "dateTime": "2026-03-03T15:00:00+11:00" },
            "end": { "dateTime": "2026-03-03T16:00:00+11:00" },
            "recurrence": ["RRULE:FREQ=WEEKLY;COUNT=3"],
        }),
    );
    api.google.put_event(
        "primary",
        json!({ "id": "lunch", "summary": "Lunch", "start": { "date": "2026-03-04" }, "end": { "date": "2026-03-05" } }),
    );
    api.sync();

    let events = api.events();
    let gym: Vec<&Value> = events
        .iter()
        .filter(|(r, _)| r.starts_with("gym_"))
        .map(|(_, v)| v)
        .collect();
    // Every Monday from 2026-01-30 up to, not including, 2027-03-01.
    let mondays = (day("2026-01-30")..day("2027-03-01"))
        .filter(|d| (d + 3).rem_euclid(7) == 0)
        .count();
    assert_eq!(gym.len(), mondays);
    let mut starts: Vec<&str> = gym
        .iter()
        .map(|e| e["fields"]["when"]["start"].as_str().unwrap())
        .collect();
    starts.sort();
    assert_eq!(starts.first(), Some(&"2026-02-02"));
    assert_eq!(starts.last(), Some(&"2027-02-22"));
    assert!(
        gym.iter()
            .all(|e| e["fields"]["series"] == "RRULE:FREQ=WEEKLY")
    );
    assert!(!events.contains_key("gym"), "the series master has no note");
    assert!(events.contains_key("gym_20260202"));

    let review = "review_20260310T040000Z";
    assert_eq!(
        events[review]["fields"]["when"],
        json!({ "start": "2026-03-10T15:00:00+11:00", "end": "2026-03-10T16:00:00+11:00" })
    );
    assert_eq!(
        events.keys().filter(|r| r.starts_with("review_")).count(),
        3
    );
    let instances = api
        .google
        .requests_to("/calendar/v3/calendars/primary/events/gym/instances");
    assert_eq!(instances.len(), 1);
    assert_eq!(
        instances[0].query("timeMin").as_deref(),
        Some("2026-01-30T00:00:00Z")
    );
    assert_eq!(
        instances[0].query("timeMax").as_deref(),
        Some("2027-03-01T00:00:00Z")
    );

    // In Google: one occurrence moved and renamed, one cancelled, a single event deleted.
    let moved_id = api.event_id(review);
    let cancelled_id = api.event_id("gym_20260309");
    let lunch_id = api.event_id("lunch");
    api.google.change_event(
        "primary",
        review,
        json!({
            "summary": "Review (moved)",
            "start": { "dateTime": "2026-03-11T15:00:00+11:00" },
            "end": { "dateTime": "2026-03-11T16:00:00+11:00" },
        }),
    );
    api.google.cancel_event("primary", "gym_20260309");
    api.google.cancel_event("primary", "lunch");
    api.google.clear_requests();
    let run = api.sync();
    assert!(
        run["log"]
            .as_str()
            .unwrap()
            .contains("0 created, 1 updated, 2 trashed"),
        "{run}"
    );
    // Only the changed occurrence and the cancels came; no series was fetched again.
    assert!(
        api.google
            .requests_to("/calendar/v3/calendars/primary/events/gym/instances")
            .is_empty()
    );

    let note = api.note(&moved_id);
    assert_eq!(note["note"]["fields"]["title"], "Review (moved)");
    assert_eq!(
        note["note"]["fields"]["when"]["start"],
        "2026-03-11T15:00:00+11:00"
    );
    assert_eq!(
        note["note"]["fields"]["series"],
        "RRULE:FREQ=WEEKLY;COUNT=3"
    );
    let trashed = api.trashed();
    assert!(trashed.contains(&cancelled_id), "{trashed:?}");
    assert!(trashed.contains(&lunch_id), "{trashed:?}");
    let events = api.events();
    assert!(!events.contains_key("gym_20260309"));
    assert_eq!(
        events.keys().filter(|r| r.starts_with("gym_")).count(),
        mondays - 1
    );

    // The whole series cancelled: every occurrence goes.
    api.google.cancel_event("primary", "review");
    api.sync();
    let events = api.events();
    assert_eq!(
        events.keys().filter(|r| r.starts_with("review_")).count(),
        0
    );
}

#[test]
fn an_incremental_run_uses_the_sync_token_and_a_410_resyncs_without_duplicates() {
    let api = Api::start();
    api.google.put_event(
        "primary",
        json!({ "id": "one", "summary": "One", "start": { "date": "2026-03-04" }, "end": { "date": "2026-03-05" } }),
    );
    api.google.put_event(
        "primary",
        json!({
            "id": "weekly",
            "summary": "Weekly",
            "start": { "date": "2026-03-02" },
            "end": { "date": "2026-03-03" },
            "recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"],
        }),
    );
    api.sync();
    let token = api.state()["sync_tokens"]["primary"]
        .as_str()
        .unwrap()
        .to_owned();

    api.google.put_event(
        "primary",
        json!({ "id": "two", "summary": "Two", "start": { "date": "2026-03-06" }, "end": { "date": "2026-03-07" } }),
    );
    api.google.clear_requests();
    api.sync();
    let lists = api
        .google
        .requests_to("/calendar/v3/calendars/primary/events");
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert_eq!(lists[0].query("syncToken"), Some(token.clone()));
    assert_eq!(api.events().len(), 2 + 4);
    let next = api.state()["sync_tokens"]["primary"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(next, token);

    // The token expires, and meanwhile one event is added and one deleted.
    api.google.expire_sync_tokens();
    api.google.put_event(
        "primary",
        json!({ "id": "three", "summary": "Three", "start": { "date": "2026-03-08" }, "end": { "date": "2026-03-09" } }),
    );
    let gone = api.event_id("one");
    api.google.cancel_event("primary", "one");
    api.google.clear_requests();
    let run = api.sync();
    assert!(run["log"].as_str().unwrap().contains("expired"), "{run}");
    let lists = api
        .google
        .requests_to("/calendar/v3/calendars/primary/events");
    let lists: Vec<_> = lists
        .iter()
        .filter(|r| !r.route().contains("/instances"))
        .collect();
    assert_eq!(lists.len(), 2, "{lists:?}");
    assert_eq!(lists[0].query("syncToken"), Some(next));
    assert_eq!(lists[1].query("syncToken"), None);

    let events = api.events(); // fails on a duplicate
    let refs: Vec<&str> = events.keys().map(String::as_str).collect();
    assert_eq!(
        refs,
        [
            "three",
            "two",
            "weekly_20260302",
            "weekly_20260309",
            "weekly_20260316",
            "weekly_20260323"
        ]
    );
    assert!(api.trashed().contains(&gone));
}

#[test]
fn the_daily_window_roll_adds_the_new_days_occurrences() {
    let api = Api::start();
    // Daily from near the window's end, with no end.
    api.google.put_event(
        "primary",
        json!({
            "id": "daily",
            "summary": "Journal",
            "start": { "date": "2027-02-20" },
            "end": { "date": "2027-02-21" },
            "recurrence": ["RRULE:FREQ=DAILY"],
        }),
    );
    api.sync();
    assert_eq!(api.events().len(), 9, "2027-02-20 to 2027-02-28");

    // A day later the window reaches 2027-03-02, so 2027-03-01 is added, and Google itself
    // reported no change.
    api.advance("P1D");
    api.google.clear_requests();
    api.sync();
    let events = api.events();
    assert_eq!(events.len(), 10);
    assert_eq!(
        events["daily_20270301"]["fields"]["when"]["start"],
        "2027-03-01"
    );
    assert_eq!(
        events["daily_20270301"]["fields"]["series"],
        "RRULE:FREQ=DAILY"
    );
    let rolled = api
        .google
        .requests_to("/calendar/v3/calendars/primary/events/daily/instances");
    assert_eq!(rolled.len(), 1);
    assert_eq!(
        rolled[0].query("timeMin").as_deref(),
        Some("2027-03-01T00:00:00Z")
    );
    assert_eq!(
        rolled[0].query("timeMax").as_deref(),
        Some("2027-03-02T00:00:00Z")
    );
    assert_eq!(api.state()["window_end"], "2027-03-02");

    // The same day again rolls nothing.
    api.google.clear_requests();
    api.sync();
    assert!(
        api.google
            .requests_to("/calendar/v3/calendars/primary/events/daily/instances")
            .is_empty()
    );
}

#[test]
fn a_note_changed_in_the_vault_and_in_google_merges_field_by_field() {
    let api = Api::start();
    let event = |id: &str, title: &str| {
        json!({
            "id": id,
            "summary": title,
            "location": "Room 1",
            "description": "Agenda",
            "start": { "dateTime": "2026-03-05T10:00:00Z" },
            "end": { "dateTime": "2026-03-05T11:00:00Z" },
        })
    };
    api.google.put_event("primary", event("newer", "Planning"));
    api.google.put_event("primary", event("older", "Retro"));
    api.sync();

    // The vault changes the title and the location of both, at the test clock's 08:00.
    let newer = api.event_id("newer");
    let older = api.event_id("older");
    api.edit(
        &newer,
        json!({ "title": "Planning (vault)", "location": "Room 2" }),
    );
    api.edit(
        &older,
        json!({ "title": "Retro (vault)", "location": "Room 2" }),
    );
    // Google changes the location and the description of both: once after the vault's change,
    // once before it.
    api.google.change_event(
        "primary",
        "newer",
        json!({ "location": "Room 3", "description": "New agenda", "updated": "2026-03-01T09:00:00.000Z" }),
    );
    api.google.change_event(
        "primary",
        "older",
        json!({ "location": "Room 3", "description": "New agenda", "updated": "2026-03-01T07:00:00.000Z" }),
    );
    api.sync();

    let note = api.note(&newer);
    let f = &note["note"]["fields"];
    assert_eq!(f["title"], "Planning (vault)", "changed only in the vault");
    assert_eq!(
        f["location"], "Room 3",
        "changed on both, Google's change is newer"
    );
    assert_eq!(
        text(&note, "description"),
        "New agenda",
        "changed only in Google"
    );
    let snapshot = &block(&note, "dict", "snapshot")["fields"];
    assert_eq!(snapshot["title"], "Planning");
    assert_eq!(snapshot["location"], "Room 3");
    assert_eq!(
        f["etag"],
        api.google.event("primary", "newer").unwrap()["etag"]
    );

    let note = api.note(&older);
    let f = &note["note"]["fields"];
    assert_eq!(f["title"], "Retro (vault)");
    assert_eq!(
        f["location"], "Room 2",
        "changed on both, the vault's change is newer"
    );
    assert_eq!(text(&note, "description"), "New agenda");
    assert_eq!(
        block(&note, "dict", "snapshot")["fields"]["location"],
        "Room 3"
    );
}

#[test]
fn invalid_grant_notifies_once_across_two_runs_and_does_not_fail_the_run() {
    let api = Api::start();
    api.google.set_invalid_grant(true);
    for _ in 0..2 {
        api.sync();
    }
    let titles: Vec<String> = api
        .notifications()
        .iter()
        .map(|n| n["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles, ["Google sign-in expired"]);
    let error = api.state()["last_error"].clone();
    assert!(
        error
            .as_str()
            .is_some_and(|e| e.starts_with("invalid_grant")),
        "{error}"
    );
    let runs = expect(
        api.req("GET", "/api/tasks/google_calendar/runs")
            .send()
            .unwrap(),
        200,
    );
    let outcomes: Vec<&Value> = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["outcome"])
        .collect();
    assert_eq!(outcomes, [&json!("succeeded"), &json!("succeeded")]);
    assert!(api.google.requests_to("/calendar/v3").is_empty());

    // Connected again: the next run syncs and clears the error.
    api.google.set_invalid_grant(false);
    api.sync();
    let f = api.state();
    assert!(f["last_error"].is_null(), "{f}");
    assert_eq!(f["notified_invalid_grant"], false, "{f}");
}

#[test]
fn each_listed_calendar_syncs_with_its_own_token_and_names_itself_on_its_notes() {
    let google = "team@group.calendar.google.com";
    let mut api = Api::bare();
    api.google = FakeGoogle::builder().calendar(google, "Team").start();
    api.put_settings(json!({ "calendars": ["primary", google] }));
    let all_day = |id: &str| json!({ "id": id, "summary": id, "start": { "date": "2026-03-04" }, "end": { "date": "2026-03-05" } });
    api.google.put_event("primary", all_day("mine"));
    api.google.put_event(google, all_day("ours"));
    api.sync();

    let events = api.events();
    assert_eq!(events["mine"]["fields"]["calendar"], "primary");
    assert_eq!(events["ours"]["fields"]["calendar"], google);
    // Each note names its calendar from the calendar list, the primary one by its own name
    // (ticket 103).
    assert_eq!(events["mine"]["fields"]["calendar_name"], "Primary");
    assert_eq!(events["ours"]["fields"]["calendar_name"], "Team");
    assert_eq!(
        api.google
            .requests_to("/calendar/v3/users/me/calendarList")
            .len(),
        1
    );
    let state = api.state();
    let mut keys: Vec<&String> = state["sync_tokens"].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["primary", google]);
    assert_eq!(
        api.google
            .requests_to("/calendar/v3/calendars/team%40group.calendar.google.com/events")
            .len(),
        1
    );
}

#[test]
fn a_gmail_run_and_a_calendar_run_each_change_only_their_own_state_and_write_no_note_history() {
    let api = Api::start();
    api.put_settings(json!({ "gmail_mode": "all" }));
    let task = std::fs::read_to_string(api.server.vault_path().join("tasks/google_gmail.wcl"))
        .unwrap()
        .replace(
            "allow_domains = [",
            &format!("allow_domains = [\"{}\", ", api.google.host()),
        );
    expect(
        api.req("PUT", "/api/tasks/google_gmail")
            .body(task)
            .send()
            .unwrap(),
        200,
    );
    api.google.put_event(
        "primary",
        json!({ "id": "one", "summary": "One", "start": { "date": "2026-03-04" }, "end": { "date": "2026-03-05" } }),
    );
    let gmail_run = || {
        let run = expect(
            api.req("POST", "/api/tasks/google_gmail/run?wait=true")
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run}");
    };
    let gmail_state = || api.server.secret_json("google_gmail_state");
    let settings = api.server.secret("google_settings");

    gmail_run();
    let gmail = gmail_state();
    assert!(gmail["history_id"].as_str().is_some(), "{gmail}");
    assert!(
        api.state().is_null(),
        "the Gmail run wrote the Calendar state"
    );

    api.sync();
    let calendar = api.state();
    assert!(calendar["sync_tokens"]["primary"].is_string(), "{calendar}");
    assert_eq!(
        gmail_state(),
        gmail,
        "the Calendar run changed the Gmail state"
    );
    assert_eq!(api.server.secret("google_settings"), settings);

    // A later pair of runs moves both states on, and no note gains a version for it.
    let versions = || {
        let state =
            state::State::open(&api.server.vault_path().join(".vault/state.db"), None).unwrap();
        state
            .live_notes()
            .unwrap()
            .iter()
            .map(|id| state.history(id).unwrap().len())
            .sum::<usize>()
    };
    let before = versions();
    api.advance("PT10M");
    gmail_run();
    let gmail_after = gmail_state();
    assert_ne!(gmail_after["last_run"], gmail["last_run"], "{gmail_after}");
    assert_eq!(api.state(), calendar);
    api.sync();
    let calendar_after = api.state();
    assert_ne!(calendar_after["last_run"], calendar["last_run"]);
    assert_eq!(gmail_state(), gmail_after);
    assert_eq!(versions(), before, "a cursor move wrote note history");
    assert_eq!(api.server.secret("google_settings"), settings);
}
