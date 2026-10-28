//! S1 with the fake Google server: event series (event-series E2, E3, E5, E6). The calendar sync
//! pulls a recurring event into an `event_series` note with its occurrences linked, the shipped
//! `google_series` handlers write series changes back to the master, and a series kept in the
//! vault gets its occurrences from the server and sends nothing to Google.
//!
//! The server runs on a test clock fixed at 2026-03-01 08:00 UTC, so the occurrence window runs
//! from 2026-01-30 to 2027-03-01.

use std::collections::BTreeMap;
use std::ffi::OsStr;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::Request;
use testkit::fake_google::{
    CALENDAR_PATH, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN, TOKEN_PATH,
};
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
    /// A server on the test clock whose calendar task and `event` and `event_series` handlers
    /// reach the fake, with the three secrets and the settings set.
    fn start() -> Api {
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
            api.put_secret(name, value);
        }
        let allow = format!("allow_domains = [\"{}\", ", api.google.host());
        let task = api.file("tasks/google_calendar.wcl");
        expect(
            api.req("PUT", "/api/tasks/google_calendar")
                .body(task.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        for (ty, script) in [("event", "google_event"), ("event_series", "google_series")] {
            let text = api.file(&format!("types/{ty}.wcl"));
            assert_eq!(
                text.matches(&format!("script        = \"{script}\""))
                    .count(),
                3,
                "{ty}"
            );
            expect(
                api.req("PUT", &format!("/api/types/{ty}"))
                    .body(text.replace("allow_domains = [", &allow))
                    .send()
                    .unwrap(),
                200,
            );
        }
        let settings = json!({ "endpoints": api.google.endpoints() });
        api.put_secret("google_settings", &settings.to_string());
        api
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

    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(path)).unwrap()
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
        self.settle();
        made["id"].as_str().unwrap().to_owned()
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notes/{id}")).send().unwrap(),
            200,
        )
    }

    fn delete(&self, id: &str) {
        let version = self.note(id)["version"].as_str().unwrap().to_owned();
        expect(
            self.req("DELETE", &format!("/api/notes/{id}"))
                .header("if-match", format!("\"{version}\""))
                .send()
                .unwrap(),
            200,
        );
        self.settle();
    }

    /// Run the calendar task now, then let every handler it set off finish.
    fn sync(&self) -> Value {
        let run = expect(
            self.req("POST", "/api/tasks/google_calendar/run?wait=true")
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run}");
        self.settle();
        run
    }

    fn settle(&self) {
        let failed = self.server.settle_handlers();
        assert!(failed.is_empty(), "{failed:#?}");
    }

    fn query(&self, text: &str) -> Vec<Value> {
        let page = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": text, "page_size": 1000 }))
                .send()
                .unwrap(),
            200,
        );
        page["rows"].as_array().unwrap().clone()
    }

    /// Every event note, by `google_ref`.
    fn events(&self) -> BTreeMap<String, Value> {
        self.query("from event include archived")
            .into_iter()
            .map(|row| {
                (
                    row["fields"]["google_ref"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    row,
                )
            })
            .collect()
    }

    /// Every series note.
    fn series(&self) -> Vec<Value> {
        self.query("from event_series include archived")
    }

    /// The occurrence notes linked to series `id`.
    fn occurrences(&self, id: &str) -> Vec<Value> {
        self.query(&format!(
            "from event where series_ref == \"{id}\" sort when.start asc"
        ))
    }

    /// Change fields of note `id` as the user would, then let its handlers finish.
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
        self.settle();
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

    /// The Calendar API requests other than reads.
    fn writes(&self) -> Vec<Request> {
        self.google
            .requests_to(CALENDAR_PATH)
            .into_iter()
            .filter(|r| r.method != "GET")
            .collect()
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

/// A timed weekly master on Mondays at 09:30 in Melbourne, five of them.
fn standup() -> Value {
    json!({
        "id": "standup",
        "summary": "Standup",
        "location": "Room 1",
        "start": { "dateTime": "2026-03-02T09:30:00+11:00", "timeZone": "Australia/Melbourne" },
        "end": { "dateTime": "2026-03-02T09:45:00+11:00", "timeZone": "Australia/Melbourne" },
        "recurrence": ["RRULE:FREQ=WEEKLY;COUNT=5"],
        "organizer": { "email": "me@example.com" },
    })
}

#[test]
fn a_recurring_master_becomes_a_series_note_with_its_occurrences_linked() {
    let api = Api::start();
    let stored = api.google.put_event("primary", standup());
    api.sync();

    let series = api.series();
    assert_eq!(series.len(), 1, "{series:?}");
    let id = series[0]["id"].as_str().unwrap().to_owned();
    let note = api.note(&id);
    let f = &note["note"]["fields"];
    assert_eq!(f["title"], "Standup");
    assert_eq!(f["rule"], json!(["RRULE:FREQ=WEEKLY;COUNT=5"]));
    assert_eq!(
        f["starts"],
        json!({ "start": "2026-03-02T09:30:00+11:00", "end": "2026-03-02T09:45:00+11:00" })
    );
    assert_eq!(f["timezone"], "Australia/Melbourne");
    assert_eq!(f["calendar"], "primary");
    assert_eq!(f["calendar_name"], "Primary");
    assert_eq!(f["location"], "Room 1");
    assert_eq!(f["google_ref"], "standup");
    assert_eq!(f["etag"], stored["etag"]);
    let snapshot = block(&note, "dict", "snapshot")["fields"].clone();
    assert_eq!(snapshot["rule"], "RRULE:FREQ=WEEKLY;COUNT=5");
    assert_eq!(snapshot["timezone"], "Australia/Melbourne");
    assert_eq!(snapshot["title"], "Standup");

    let occ = api.occurrences(&id);
    assert_eq!(occ.len(), 5);
    assert_eq!(occ[0]["fields"]["google_ref"], "standup_20260301T223000Z");
    assert!(
        occ.iter()
            .all(|o| o["fields"]["series"] == "RRULE:FREQ=WEEKLY;COUNT=5")
    );
    // Every note the sync made is in step: nothing went back to Google.
    assert!(api.writes().is_empty(), "{:?}", api.writes());
}

#[test]
fn a_changed_master_updates_the_series_and_a_cancelled_one_trashes_it_and_its_occurrences() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.series()[0]["id"].as_str().unwrap().to_owned();

    api.google.change_event(
        "primary",
        "standup",
        json!({ "summary": "Daily standup", "recurrence": ["RRULE:FREQ=WEEKLY;COUNT=3"] }),
    );
    api.sync();
    let f = api.note(&id)["note"]["fields"].clone();
    assert_eq!(f["title"], "Daily standup");
    assert_eq!(f["rule"], json!(["RRULE:FREQ=WEEKLY;COUNT=3"]));
    let occ = api.occurrences(&id);
    assert_eq!(occ.len(), 3);
    assert!(occ.iter().all(|o| o["fields"]["title"] == "Daily standup"));

    let ids: Vec<String> = occ
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    api.google.cancel_event("primary", "standup");
    api.google.clear_requests();
    api.sync();
    assert!(api.series().is_empty());
    assert!(api.events().is_empty());
    let trashed = api.trashed();
    assert!(trashed.contains(&id));
    assert!(ids.iter().all(|o| trashed.contains(o)));
}

#[test]
fn occurrences_from_before_series_notes_are_linked_and_their_master_fetched() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    // An older vault: occurrence notes as the sync made them before series notes, and a sync
    // token that lists nothing new, so the master must be fetched to make its series note.
    let script = r#"use vault

fn run() -> Result[string, string] {
    for at in ["20260301T223000Z", "20260308T223000Z"] {
        let day = if at == "20260301T223000Z" { "2026-03-02" } else { "2026-03-09" }
        let start = day + "T09:30:00+11:00"
        let end = day + "T09:45:00+11:00"
        vault::create("event", Value::Map(#{
            "fields": Value::Map(#{
                "title": Value::String("Standup"),
                "when": Value::Map(#{ "start": Value::String(start), "end": Value::String(end) }),
                "location": Value::String("Room 1"),
                "organiser": Value::String("me@example.com"),
                "status": Value::String("confirmed"),
                "calendar": Value::String("primary"),
                "series": Value::String("RRULE:FREQ=WEEKLY;COUNT=5"),
                "google_ref": Value::String("standup_" + at),
                "etag": Value::String("\"1\""),
            }),
            "blocks": Value::List([Value::Map(#{
                "kind": Value::String("dict"),
                "labels": Value::List([Value::String("snapshot")]),
                "fields": Value::Map(#{
                    "title": Value::String("Standup"),
                    "start": Value::String(start),
                    "end": Value::String(end),
                    "location": Value::String("Room 1"),
                    "description": Value::String(""),
                    "attendees": Value::String(""),
                    "organiser": Value::String("me@example.com"),
                    "status": Value::String("confirmed"),
                    "meet_url": Value::String(""),
                }),
            })]),
        }))?;
    }
    Ok("made")
}
"#;
    expect(
        api.req("PUT", "/api/scripts/old_occurrences")
            .json(&json!({ "source": script }))
            .send()
            .unwrap(),
        200,
    );
    expect(
        api.req("PUT", "/api/tasks/old_occurrences")
            .body("schedule = \"0 0 1 1 *\"\nscript = \"old_occurrences\"\n")
            .send()
            .unwrap(),
        200,
    );
    // Made while Google is not connected, the notes' handlers send nothing.
    expect(
        api.req("DELETE", "/api/secrets/google_refresh_token")
            .send()
            .unwrap(),
        200,
    );
    let run = expect(
        api.req("POST", "/api/tasks/old_occurrences/run?wait=true")
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(run["outcome"], "succeeded", "{run}");
    api.settle();
    api.put_secret("google_refresh_token", DEFAULT_REFRESH_TOKEN);
    assert!(api.series().is_empty());
    api.put_secret(
        "google_calendar_state",
        &json!({ "sync_tokens": { "primary": "sync_1_1" }, "window_end": "2027-03-01" })
            .to_string(),
    );
    api.google.clear_requests();

    api.sync();
    let series = api.series();
    assert_eq!(series.len(), 1);
    let id = series[0]["id"].as_str().unwrap().to_owned();
    let gets = api
        .google
        .requests_to(&format!("{CALENDAR_PATH}/calendars/primary/events/standup"))
        .into_iter()
        .filter(|r| r.method == "GET" && r.route().ends_with("/standup"))
        .count();
    assert_eq!(gets, 1);
    let linked: Vec<String> = api
        .occurrences(&id)
        .iter()
        .map(|o| o["fields"]["google_ref"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        linked,
        ["standup_20260301T223000Z", "standup_20260308T223000Z"]
    );
    // Linking sent nothing to Google, and a second run has nothing left to link.
    assert!(api.writes().is_empty(), "{:?}", api.writes());
    let run = api.sync();
    assert!(
        run["log"]
            .as_str()
            .unwrap()
            .contains("0 created, 0 updated, 0 trashed"),
        "{run}"
    );
}

#[test]
fn editing_a_series_patches_the_master_with_only_the_changed_groups() {
    let api = Api::start();
    let stored = api.google.put_event("primary", standup());
    api.sync();
    let id = api.series()[0]["id"].as_str().unwrap().to_owned();
    let occ_before = api.occurrences(&id);
    api.google.clear_requests();

    api.edit(
        &id,
        json!({ "title": "Team standup", "rule": ["RRULE:FREQ=WEEKLY;COUNT=4"] }),
    );
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    let patch = &writes[0];
    assert_eq!(patch.method, "PATCH");
    assert_eq!(
        patch.route(),
        format!("{CALENDAR_PATH}/calendars/primary/events/standup")
    );
    assert_eq!(patch.header("if-match"), stored["etag"].as_str());
    assert_eq!(
        patch.json(),
        json!({ "summary": "Team standup", "recurrence": ["RRULE:FREQ=WEEKLY;COUNT=4"] })
    );
    // One token refresh for the edit; the write-back sent nothing.
    assert_eq!(api.google.requests_to(TOKEN_PATH).len(), 1);
    let note = api.note(&id);
    let master = api.google.event("primary", "standup").unwrap();
    assert_eq!(note["note"]["fields"]["etag"], master["etag"]);
    assert_eq!(
        block(&note, "dict", "snapshot")["fields"]["title"],
        "Team standup"
    );
    // The occurrences are not written by the edit: the next sync brings Google's change.
    assert_eq!(api.occurrences(&id), occ_before);
    api.sync();
    let occ = api.occurrences(&id);
    assert_eq!(occ.len(), 4);
    assert!(occ.iter().all(|o| o["fields"]["title"] == "Team standup"));
    // The sync sent no change back; the fifth occurrence, gone from Google, went to the trash,
    // whose handler's DELETE Google answers as already done.
    let sent: Vec<Request> = api
        .writes()
        .into_iter()
        .filter(|r| r.method != "DELETE")
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
}

#[test]
fn a_series_changed_in_google_since_the_snapshot_is_merged_after_a_412() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.series()[0]["id"].as_str().unwrap().to_owned();
    // Google moves the series' room; the vault, not yet synced, renames it.
    api.google
        .change_event("primary", "standup", json!({ "location": "Room 2" }));
    let current = api.google.event("primary", "standup").unwrap();
    api.google.clear_requests();

    api.edit(&id, json!({ "title": "Team standup" }));
    let writes = api.writes();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(writes[1].header("if-match"), current["etag"].as_str());
    assert_eq!(writes[1].json(), json!({ "summary": "Team standup" }));
    let master = api.google.event("primary", "standup").unwrap();
    assert_eq!(master["summary"], "Team standup");
    assert_eq!(master["location"], "Room 2");
    let f = api.note(&id)["note"]["fields"].clone();
    assert_eq!(f["title"], "Team standup");
    assert_eq!(f["location"], "Room 2");
    assert_eq!(f["etag"], master["etag"]);
}

#[test]
fn creating_a_series_with_a_calendar_inserts_a_recurring_event_and_the_sync_brings_its_occurrences()
{
    let api = Api::start();
    let id = api.create_note(
        "event_series",
        json!({ "fields": {
            "title": "Review",
            "rule": ["RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=3"],
            "starts": { "start": "2026-03-03T15:00:00+11:00", "end": "2026-03-03T16:00:00+11:00" },
            "timezone": "Australia/Melbourne",
            "calendar": "primary",
        }}),
    );
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "POST");
    assert_eq!(
        writes[0].json(),
        json!({
            "summary": "Review",
            "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=3"],
            "attendees": [],
            "start": { "dateTime": "2026-03-03T15:00:00+11:00", "date": null, "timeZone": "Australia/Melbourne" },
            "end": { "dateTime": "2026-03-03T16:00:00+11:00", "date": null, "timeZone": "Australia/Melbourne" },
        })
    );
    let note = api.note(&id);
    let master_id = note["note"]["fields"]["google_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let master = api.google.event("primary", &master_id).unwrap();
    assert_eq!(note["note"]["fields"]["etag"], master["etag"]);
    assert_eq!(
        block(&note, "dict", "snapshot")["fields"]["rule"],
        "RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=3"
    );
    // A Google series makes no occurrences in the vault: the sync brings Google's.
    assert!(api.occurrences(&id).is_empty());
    api.sync();
    assert_eq!(api.series().len(), 1);
    let occ = api.occurrences(&id);
    assert_eq!(occ.len(), 3);
    assert_eq!(
        occ[0]["fields"]["google_ref"],
        json!(format!("{master_id}_20260303T040000Z"))
    );
    assert_eq!(api.writes().len(), 1, "{:?}", api.writes());
}

#[test]
fn deleting_a_series_deletes_the_master_and_sends_nothing_for_its_occurrences() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.series()[0]["id"].as_str().unwrap().to_owned();
    let occ: Vec<String> = api
        .occurrences(&id)
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    api.google.clear_requests();

    api.delete(&id);
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "DELETE");
    assert_eq!(
        writes[0].route(),
        format!("{CALENDAR_PATH}/calendars/primary/events/standup")
    );
    // The occurrences went with it, in the same write.
    let trashed = api.trashed();
    assert!(occ.iter().all(|o| trashed.contains(o)));
    assert!(api.events().is_empty());
    assert_eq!(
        api.google.event("primary", "standup").unwrap()["status"],
        "cancelled"
    );
}

#[test]
fn a_series_kept_in_the_vault_makes_its_occurrences_rolls_forward_and_sends_nothing() {
    let api = Api::start();
    let id = api.create_note(
        "event_series",
        json!({ "fields": {
            "title": "Gym",
            "rule": ["RRULE:FREQ=WEEKLY;BYDAY=MO"],
            "starts": { "start": "2026-03-02", "end": "2026-03-02" },
        }}),
    );
    let occ = api.occurrences(&id);
    // Every Monday from 2 March 2026 to 22 February 2027.
    assert_eq!(occ.len(), 52);
    assert_eq!(occ[0]["fields"]["when"]["start"], "2026-03-02");
    assert!(occ.iter().all(|o| o["fields"].get("google_ref").is_none()));

    // Editing and deleting occurrences, and editing the series, send nothing.
    let second = occ[1]["id"].as_str().unwrap().to_owned();
    api.edit(&second, json!({ "location": "Pool" }));
    let third = occ[2]["id"].as_str().unwrap().to_owned();
    api.delete(&third);
    assert_eq!(
        api.note(&id)["note"]["fields"]["rule"],
        json!(["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE;VALUE=DATE:20260316"])
    );
    api.edit(&id, json!({ "location": "Gym hall" }));
    assert_eq!(api.occurrences(&id).len(), 51);

    // A week on, the window holds one more Monday.
    expect(
        api.req("POST", "/api/test/clock")
            .json(&json!({ "advance": "P7D" }))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
    let occ = api.occurrences(&id);
    assert_eq!(occ.len(), 52);
    assert_eq!(occ.last().unwrap()["fields"]["when"]["start"], "2027-03-01");
    assert!(
        api.google.requests().is_empty(),
        "{:?}",
        api.google.requests()
    );
}

#[test]
fn a_series_rule_that_does_not_parse_is_refused() {
    let api = Api::start();
    let res = expect(
        api.req("POST", "/api/notes?type=event_series")
            .json(&json!({ "note": { "fields": {
                "title": "Gym",
                "rule": ["RRULE:FREQ=WEEKLY", "EXDATE:sometime"],
                "starts": { "start": "2026-03-02", "end": "2026-03-02" },
            }}}))
            .send()
            .unwrap(),
        400,
    );
    assert_eq!(res["error"], "invalid", "{res}");
    assert_eq!(res["issues"][0]["at"], "rule", "{res}");
    let message = res["issues"][0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("line 2, \"EXDATE:sometime\", has a date that does not read"),
        "{message}"
    );
}
