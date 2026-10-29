//! S1 with the fake Google server: the shipped `event` type's `google_event` handlers writing
//! vault changes back to Google Calendar (google-sync D9, D10, D20). Vault edits go through the
//! notes API; what was sent is read from the requests the fake received.
//!
//! The server runs on a test clock fixed at 2026-03-01 08:00 UTC, so a note's `updated` is that
//! time unless a test moves the clock on.

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
    /// A server on the test clock whose `google_calendar` task and `event` handlers reach
    /// `google`, with the three secrets set and the `google_settings` secret pointing at the fake,
    /// holding `settings` besides.
    fn start_with(google: FakeGoogle, settings: Value) -> Api {
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
        // The shipped task and handlers, with the fake's host allowed as the spec's tests do.
        let allow = format!("allow_domains = [\"{}\", ", api.google.host());
        let task = api.file("tasks/google_calendar.wcl");
        expect(
            api.req("PUT", "/api/tasks/google_calendar")
                .body(task.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        let event_type = api.file("types/event.wcl");
        assert_eq!(
            event_type
                .matches("script        = \"google_event\"")
                .count(),
            3
        );
        expect(
            api.req("PUT", "/api/types/event")
                .body(event_type.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        let mut settings = settings;
        settings["endpoints"] = api.google.endpoints();
        expect(
            api.req("PUT", "/api/secrets/google_settings")
                .json(&json!({ "value": settings.to_string() }))
                .send()
                .unwrap(),
            200,
        );
        api
    }

    fn start() -> Api {
        Api::start_with(FakeGoogle::start(), json!({}))
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
        made["id"].as_str().unwrap().to_owned()
    }

    fn note(&self, id: &str) -> Value {
        expect(
            self.req("GET", &format!("/api/notes/{id}")).send().unwrap(),
            200,
        )
    }

    /// Run the task now, then let every handler it set off finish.
    fn sync(&self) {
        let run = expect(
            self.req("POST", "/api/tasks/google_calendar/run?wait=true")
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(run["outcome"], "succeeded", "{run}");
        self.settle();
    }

    /// Let every queued handler run finish, and fail on any that failed.
    fn settle(&self) {
        let failed = self.server.settle_handlers();
        assert!(failed.is_empty(), "{failed:#?}");
    }

    /// Every event note that is not in the trash, by `google_ref`.
    fn events(&self) -> BTreeMap<String, Value> {
        let page = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": "from event include archived", "page_size": 1000 }))
                .send()
                .unwrap(),
            200,
        );
        let mut out = BTreeMap::new();
        for row in page["rows"].as_array().unwrap() {
            let r = row["fields"]["google_ref"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            assert!(
                out.insert(r.clone(), row.clone()).is_none(),
                "two notes for {r}"
            );
        }
        out
    }

    fn event_id(&self, google_ref: &str) -> String {
        self.events()[google_ref]["id"].as_str().unwrap().to_owned()
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

    fn history_len(&self, id: &str) -> usize {
        expect(
            self.req("GET", &format!("/api/notes/{id}/history"))
                .send()
                .unwrap(),
            200,
        )["versions"]
            .as_array()
            .unwrap()
            .len()
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

fn text(note: &Value, name: &str) -> String {
    block(note, "content", name)["blocks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["fields"]["text"].as_str().unwrap_or("").to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The synced fields of note `note` as its snapshot holds them.
fn note_as_snapshot(note: &Value) -> Value {
    let f = &note["note"]["fields"];
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let attendees: Vec<String> = f["attendees_raw"]
        .as_array()
        .into_iter()
        .flatten()
        .map(s)
        .collect();
    json!({
        "title": s(&f["title"]),
        "start": s(&f["when"]["start"]),
        "end": s(&f["when"]["end"]),
        "location": s(&f["location"]),
        "description": html_blocks::blocks_to_html(
            block(note, "content", "description")["blocks"]
                .as_array()
                .map_or(&[][..], Vec::as_slice)
        ),
        "attendees": attendees.join("\n"),
        "organiser": s(&f["organiser"]),
        "status": s(&f["status"]),
        "meet_url": s(&f["meet_url"]),
        "converter": "2",
    })
}

/// The note's synced fields equal its snapshot, and its etag is Google's.
fn assert_in_step(api: &Api, note: &Value, calendar: &str) {
    let f = &note["note"]["fields"];
    assert_eq!(
        block(note, "dict", "snapshot")["fields"],
        note_as_snapshot(note),
        "{note}"
    );
    let google_ref = f["google_ref"].as_str().unwrap();
    let event = api.google.event(calendar, google_ref).unwrap();
    assert_eq!(f["etag"], event["etag"], "{note}");
}

fn standup() -> Value {
    json!({
        "id": "standup",
        "summary": "Standup",
        "location": "Room 1",
        "description": "Agenda",
        "start": { "dateTime": "2026-03-05T10:00:00Z" },
        "end": { "dateTime": "2026-03-05T10:15:00Z" },
        "attendees": [{ "email": "ann@example.com" }],
        "organizer": { "email": "me@example.com" },
    })
}

#[test]
fn editing_a_title_patches_only_the_title_with_if_match_and_the_follow_up_sends_nothing() {
    let api = Api::start();
    let stored = api.google.put_event("primary", standup());
    api.sync();
    let id = api.event_id("standup");
    // The task made the note: its after_create sent nothing.
    assert!(api.writes().is_empty(), "{:?}", api.writes());
    api.google.clear_requests();
    let versions = api.history_len(&id);

    api.edit(&id, json!({ "title": "Daily standup" }));

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    let patch = &writes[0];
    assert_eq!(patch.method, "PATCH");
    assert_eq!(
        patch.route(),
        format!("{CALENDAR_PATH}/calendars/primary/events/standup")
    );
    assert_eq!(patch.header("if-match"), stored["etag"].as_str());
    assert_eq!(patch.json(), json!({ "summary": "Daily standup" }));
    // One token refresh for the edit's run; the write-back's own run sent nothing at all.
    assert_eq!(api.google.requests_to(TOKEN_PATH).len(), 1);
    assert_eq!(
        api.google.requests().len(),
        2,
        "{:?}",
        api.google.requests()
    );

    let event = api.google.event("primary", "standup").unwrap();
    assert_eq!(event["summary"], "Daily standup");
    assert_eq!(event["location"], "Room 1");
    let note = api.note(&id);
    assert_eq!(note["note"]["fields"]["etag"], event["etag"]);
    assert_eq!(
        block(&note, "dict", "snapshot")["fields"]["title"],
        "Daily standup"
    );
    assert_in_step(&api, &note, "primary");
    // The edit and the handler's write-back, nothing more.
    assert_eq!(api.history_len(&id), versions + 2);

    // The next sync finds Google's event as the note has it.
    api.google.clear_requests();
    api.sync();
    assert!(api.writes().is_empty(), "{:?}", api.writes());
    assert_eq!(api.history_len(&id), versions + 2);
}

#[test]
fn a_change_the_task_writes_sends_nothing_back() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.event_id("standup");
    api.google.clear_requests();

    api.google.change_event(
        "primary",
        "standup",
        json!({ "summary": "Standup (moved)", "location": "Room 9", "start": { "dateTime": "2026-03-05T11:00:00Z" }, "end": { "dateTime": "2026-03-05T11:15:00Z" } }),
    );
    api.sync();

    let note = api.note(&id);
    assert_eq!(note["note"]["fields"]["title"], "Standup (moved)");
    assert_eq!(note["note"]["fields"]["location"], "Room 9");
    assert_in_step(&api, &note, "primary");
    // Only the task's own requests: its token refresh and its listing.
    assert!(api.writes().is_empty(), "{:?}", api.writes());
    assert_eq!(api.google.requests_to(TOKEN_PATH).len(), 1);
}

#[test]
fn a_412_merges_field_by_field_and_retries_once() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.event_id("standup");
    let synced_etag = api.note(&id)["note"]["fields"]["etag"].clone();

    // Google changes the description and the location before the vault's edit (07:00, the vault's
    // is at 08:00), and the task has not run since.
    let changed = api.google.change_event(
        "primary",
        "standup",
        json!({ "description": "New agenda", "location": "Room 3", "updated": "2026-03-01T07:00:00.000Z" }),
    );
    api.google.clear_requests();
    // The vault changes the title and the location.
    api.edit(
        &id,
        json!({ "title": "Daily standup", "location": "Room 2" }),
    );

    let calls: Vec<Request> = api.google.requests_to(CALENDAR_PATH);
    let methods: Vec<&str> = calls.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["PATCH", "GET", "PATCH"], "{calls:?}");
    assert_eq!(calls[0].header("if-match"), synced_etag.as_str());
    assert_eq!(
        calls[0].json(),
        json!({ "summary": "Daily standup", "location": "Room 2" })
    );
    // The retry carries the etag Google answered with, and only what differs from Google after
    // the merge: the title changed only in the vault, the location changed on both with the
    // vault's change newer.
    assert_eq!(calls[2].header("if-match"), changed["etag"].as_str());
    assert_eq!(
        calls[2].json(),
        json!({ "summary": "Daily standup", "location": "Room 2" })
    );

    let event = api.google.event("primary", "standup").unwrap();
    assert_eq!(event["summary"], "Daily standup");
    assert_eq!(event["location"], "Room 2");
    assert_eq!(event["description"], "New agenda");
    let note = api.note(&id);
    let f = &note["note"]["fields"];
    assert_eq!(f["title"], "Daily standup");
    assert_eq!(f["location"], "Room 2");
    assert_eq!(
        text(&note, "description"),
        "New agenda",
        "changed only in Google"
    );
    assert_in_step(&api, &note, "primary");

    // A Google change newer than the vault's wins the field on both sides.
    api.google.change_event(
        "primary",
        "standup",
        json!({ "location": "Room 4", "updated": "2026-03-01T09:00:00.000Z" }),
    );
    api.google.clear_requests();
    api.edit(
        &id,
        json!({ "location": "Room 5", "title": "Standup again" }),
    );
    let calls = api.google.requests_to(CALENDAR_PATH);
    let methods: Vec<&str> = calls.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["PATCH", "GET", "PATCH"], "{calls:?}");
    assert_eq!(calls[2].json(), json!({ "summary": "Standup again" }));
    let note = api.note(&id);
    assert_eq!(note["note"]["fields"]["location"], "Room 4");
    assert_eq!(note["note"]["fields"]["title"], "Standup again");
    assert_in_step(&api, &note, "primary");
}

#[test]
fn changing_the_attendees_keeps_googles_entry_for_each_address_it_keeps() {
    let api = Api::start();
    let ann =
        json!({ "email": "ann@example.com", "displayName": "Ann", "responseStatus": "accepted" });
    let bob =
        json!({ "email": "bob@example.com", "optional": true, "responseStatus": "tentative" });
    let me = json!({ "email": "me@example.com", "organizer": true, "self": true, "responseStatus": "accepted" });
    let mut event = standup();
    event["attendees"] = json!([ann.clone(), bob, me.clone()]);
    api.google.put_event("primary", event);
    api.sync();
    let id = api.event_id("standup");
    api.google.clear_requests();

    // The vault drops Bob, adds Cy, and writes Ann's address in another case.
    api.edit(
        &id,
        json!({ "attendees_raw": ["ANN@example.com", "me@example.com", "cy@example.com"] }),
    );

    let calls = api.google.requests_to(CALENDAR_PATH);
    let methods: Vec<&str> = calls.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["GET", "PATCH"], "{calls:?}");
    let sent = json!([ann.clone(), me.clone(), { "email": "cy@example.com" }]);
    assert_eq!(calls[1].json(), json!({ "attendees": sent }));
    let event = api.google.event("primary", "standup").unwrap();
    assert_eq!(event["attendees"], sent);
    // The note takes Google's addresses and is in step, so the write-back sends nothing more.
    let note = api.note(&id);
    assert_eq!(
        note["note"]["fields"]["attendees_raw"],
        json!(["ann@example.com", "me@example.com", "cy@example.com"])
    );
    assert_in_step(&api, &note, "primary");

    // After a 412 the merge keeps the entries of the event as Google now holds it.
    let cy = json!({ "email": "cy@example.com", "responseStatus": "declined" });
    api.google.change_event(
        "primary",
        "standup",
        json!({ "attendees": [ann.clone(), me.clone(), cy.clone()], "updated": "2026-03-01T07:00:00.000Z" }),
    );
    api.google.clear_requests();
    api.edit(
        &id,
        json!({ "attendees_raw": ["cy@example.com", "ann@example.com"] }),
    );
    let calls = api.google.requests_to(CALENDAR_PATH);
    let methods: Vec<&str> = calls.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["GET", "PATCH", "GET", "PATCH"], "{calls:?}");
    assert_eq!(calls[3].json(), json!({ "attendees": [cy, ann] }));
    assert_in_step(&api, &api.note(&id), "primary");
}

#[test]
fn creating_an_event_note_inserts_it_into_its_calendar_or_the_default_and_stores_google_ref() {
    let google = FakeGoogle::builder()
        .calendar("work@example.com", "Work")
        .start();
    let api = Api::start_with(google, json!({ "default_calendar": "work@example.com" }));

    // A note naming its calendar, all day over two days.
    let trip = api.create_note(
        "event",
        json!({
            "fields": {
                "title": "Trip",
                "when": { "start": "2026-03-10", "end": "2026-03-11" },
                "calendar": "primary",
                "location": "Sydney",
            },
            "blocks": [{ "kind": "content", "labels": ["description"], "blocks": [
                { "kind": "paragraph", "fields": { "text": "Flights booked" } },
                { "kind": "paragraph", "fields": { "text": "Hotel next" } },
            ] }],
        }),
    );
    // A timed note with no calendar goes to the default calendar.
    let review = api.create_note(
        "event",
        json!({
            "fields": {
                "title": "Review",
                "when": { "start": "2026-03-12T15:00:00+11:00", "end": "2026-03-12T16:00:00+11:00" },
                "attendees_raw": ["ann@example.com"],
            },
        }),
    );
    // A note with no time yet is not sent.
    let idea = api.create_note("event", json!({ "fields": { "title": "Some day" } }));
    api.settle();

    let writes = api.writes();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert!(writes.iter().all(|r| r.method == "POST"), "{writes:?}");
    assert_eq!(
        writes[0].route(),
        format!("{CALENDAR_PATH}/calendars/primary/events")
    );
    assert_eq!(
        writes[0].json(),
        json!({
            "summary": "Trip",
            "start": { "date": "2026-03-10", "dateTime": null },
            "end": { "date": "2026-03-12", "dateTime": null },
            "location": "Sydney",
            "description": "<p>Flights booked</p><p>Hotel next</p>",
            "attendees": [],
        })
    );

    let note = api.note(&trip);
    let f = &note["note"]["fields"];
    let google_ref = f["google_ref"].as_str().unwrap().to_owned();
    assert!(!google_ref.is_empty());
    assert_eq!(f["calendar"], "primary");
    assert_eq!(f["status"], "confirmed", "Google's default status");
    let event = api.google.event("primary", &google_ref).unwrap();
    assert_eq!(event["summary"], "Trip");
    assert_eq!(event["end"]["date"], "2026-03-12");
    assert_in_step(&api, &note, "primary");

    let note = api.note(&review);
    let f = &note["note"]["fields"];
    assert_eq!(f["calendar"], "work@example.com");
    let review_ref = f["google_ref"].as_str().unwrap().to_owned();
    let event = api.google.event("work@example.com", &review_ref).unwrap();
    assert_eq!(event["start"]["dateTime"], "2026-03-12T15:00:00+11:00");
    assert_eq!(event["attendees"], json!([{ "email": "ann@example.com" }]));
    assert!(api.google.event("primary", &review_ref).is_none());
    assert_in_step(&api, &note, "work@example.com");

    assert!(
        api.note(&idea)["note"]["fields"]
            .get("google_ref")
            .is_none()
    );

    // Giving it a time later sends it.
    api.google.clear_requests();
    api.edit(
        &idea,
        json!({ "when": { "start": "2026-04-01", "end": "2026-04-01" } }),
    );
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(
        writes[0].route(),
        format!("{CALENDAR_PATH}/calendars/work%40example.com/events")
    );
    let note = api.note(&idea);
    assert!(note["note"]["fields"]["google_ref"].is_string(), "{note}");
    assert_in_step(&api, &note, "work@example.com");
}

#[test]
fn deleting_deletes_in_google_and_restoring_recreates_it_with_a_new_google_ref() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.event_id("standup");
    api.google.clear_requests();

    let version = api.note(&id)["version"].as_str().unwrap().to_owned();
    expect(
        api.req("DELETE", &format!("/api/notes/{id}"))
            .header("if-match", format!("\"{version}\""))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "DELETE");
    assert_eq!(
        writes[0].route(),
        format!("{CALENDAR_PATH}/calendars/primary/events/standup")
    );
    assert_eq!(
        api.google.event("primary", "standup").unwrap()["status"],
        "cancelled"
    );

    // Restored a minute later: Google's event is gone, so it is created again (D20).
    api.advance("PT1M");
    api.google.clear_requests();
    expect(
        api.req("POST", &format!("/api/trash/{id}/restore"))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
    let calls = api.google.requests_to(CALENDAR_PATH);
    let methods: Vec<&str> = calls.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["GET", "POST"], "{calls:?}");
    let note = api.note(&id);
    let f = &note["note"]["fields"];
    let google_ref = f["google_ref"].as_str().unwrap();
    assert_ne!(google_ref, "standup");
    let event = api.google.event("primary", google_ref).unwrap();
    assert_eq!(event["summary"], "Standup");
    assert_eq!(event["status"], "confirmed");
    assert_eq!(event["start"]["dateTime"], "2026-03-05T10:00:00Z");
    assert_in_step(&api, &note, "primary");

    // The next sync sees the old event cancelled and the new one as the note has it: the note
    // stays, and nothing is sent.
    api.google.clear_requests();
    api.sync();
    let events = api.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[google_ref]["id"], json!(id));
    assert!(api.writes().is_empty(), "{:?}", api.writes());
}

#[test]
fn restoring_a_note_whose_event_google_still_has_sends_nothing() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.event_id("standup");
    // Deleted in the vault, which deletes it in Google too.
    api.google.clear_requests();
    let version = api.note(&id)["version"].as_str().unwrap().to_owned();
    expect(
        api.req("DELETE", &format!("/api/notes/{id}"))
            .header("if-match", format!("\"{version}\""))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
    // Google brings the event back, as undoing a delete in Google does.
    api.google
        .change_event("primary", "standup", json!({ "status": "confirmed" }));
    api.advance("PT1M");
    api.google.clear_requests();
    expect(
        api.req("POST", &format!("/api/trash/{id}/restore"))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
    let calls = api.google.requests_to(CALENDAR_PATH);
    let methods: Vec<&str> = calls.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["GET"], "{calls:?}");
    assert_eq!(api.note(&id)["note"]["fields"]["google_ref"], "standup");
}

#[test]
fn editing_an_occurrence_patches_only_that_instance() {
    let api = Api::start();
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
    api.sync();
    let second = "review_20260310T040000Z";
    let id = api.event_id(second);
    let master_before = api.google.event("primary", "review").unwrap();
    let etag = api.note(&id)["note"]["fields"]["etag"].clone();
    api.google.clear_requests();

    api.edit(
        &id,
        json!({ "title": "Review (quarterly)", "location": "Board room" }),
    );

    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "PATCH");
    assert_eq!(
        writes[0].route(),
        format!("{CALENDAR_PATH}/calendars/primary/events/{second}")
    );
    assert_eq!(writes[0].header("if-match"), etag.as_str());
    assert_eq!(
        writes[0].json(),
        json!({ "summary": "Review (quarterly)", "location": "Board room" })
    );
    let changed = api.google.event("primary", second).unwrap();
    assert_eq!(changed["summary"], "Review (quarterly)");
    assert_eq!(
        api.google.event("primary", "review").unwrap(),
        master_before
    );
    for other in ["review_20260303T040000Z", "review_20260317T040000Z"] {
        assert_eq!(
            api.google.event("primary", other).unwrap()["summary"],
            "Review",
            "{other}"
        );
    }
    let note = api.note(&id);
    assert_eq!(
        note["note"]["fields"]["series"],
        "RRULE:FREQ=WEEKLY;COUNT=3"
    );
    assert_in_step(&api, &note, "primary");

    // The next sync brings the changed occurrence back as the note has it, and sends nothing.
    api.google.clear_requests();
    api.sync();
    assert!(api.writes().is_empty(), "{:?}", api.writes());
    assert_eq!(
        api.note(&id)["note"]["fields"]["title"],
        "Review (quarterly)"
    );
}

// ---- rich descriptions (rich-import H4, H8) ----------------------------------------------------

/// Replace note `id` whole with `note`, as the user's editor would, then let its handlers finish.
fn put_note(api: &Api, id: &str, note: Value) {
    let version = api.note(id)["version"].clone();
    expect(
        api.req("PUT", &format!("/api/notes/{id}"))
            .json(&json!({ "note": note, "base_version": version }))
            .send()
            .unwrap(),
        200,
    );
    api.settle();
}

#[test]
fn an_html_description_becomes_blocks_and_an_untouched_one_is_never_sent_back() {
    let api = Api::start();
    let mut event = standup();
    event["description"] = json!(
        "<b>Bring</b> snacks &amp; *stars* [x]<br><ul><li>chips</li><li>dip</li></ul><a href=\"https://maps.example.com/r\">the map</a>"
    );
    api.google.put_event("primary", event);
    api.sync();
    let id = api.event_id("standup");
    let note = api.note(&id);
    assert_eq!(
        block(&note, "content", "description")["blocks"],
        json!([
            { "kind": "paragraph", "fields": { "text": "**Bring** snacks & \\*stars\\* \\[x\\]" } },
            { "kind": "list", "fields": { "style": "bullet" }, "blocks": [
                { "kind": "item", "fields": { "text": "chips" } },
                { "kind": "item", "fields": { "text": "dip" } },
            ] },
            { "kind": "paragraph", "fields": { "text": "[the map](https://maps.example.com/r)" } },
        ])
    );
    assert_in_step(&api, &note, "primary");
    assert!(api.writes().is_empty(), "{:?}", api.writes());

    // A title edit sends the title alone: the description compares as itself.
    api.google.clear_requests();
    api.edit(&id, json!({ "title": "Daily standup" }));
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].json(), json!({ "summary": "Daily standup" }));
    let note = api.note(&id);
    assert_in_step(&api, &note, "primary");
    // Nor does the next sync change the note.
    let version = note["version"].clone();
    api.google.clear_requests();
    api.sync();
    assert_eq!(api.note(&id)["version"], version);
    assert!(api.writes().is_empty(), "{:?}", api.writes());
}

#[test]
fn an_edited_description_is_sent_as_html_and_rich_blocks_stay_when_google_keeps_it() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.event_id("standup");
    let mut note = api.note(&id)["note"].clone();
    let blocks = note["blocks"].as_array_mut().unwrap();
    for b in blocks.iter_mut() {
        if b["kind"] == "content" && b["labels"] == json!(["description"]) {
            b["blocks"] = json!([
                { "kind": "heading", "fields": { "level": 2, "text": "Plan" } },
                { "kind": "paragraph", "fields": { "text": "Bring **snacks** < 5 & {red}more{/}" } },
            ]);
        }
    }
    api.google.clear_requests();
    put_note(&api, &id, note);
    let writes = api.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(
        writes[0].json(),
        json!({ "description": "<p><b>Plan</b></p><p>Bring <b>snacks</b> &lt; 5 &amp; more</p>" })
    );
    let note = api.note(&id);
    // Google keeps what was sent, so the note keeps its heading and colour.
    assert_eq!(
        block(&note, "content", "description")["blocks"][0]["kind"],
        "heading"
    );
    assert_in_step(&api, &note, "primary");
    api.google.clear_requests();
    api.sync();
    assert!(api.writes().is_empty(), "{:?}", api.writes());
}

#[test]
fn an_older_snapshot_is_converted_on_the_next_sync_without_sending_anything() {
    let api = Api::start();
    let mut event = standup();
    event["description"] = json!("<b>Agenda</b><br>one");
    api.google.put_event("primary", event);
    api.sync();
    let id = api.event_id("standup");
    // The note as the older sync left it: the description's raw text a line per paragraph, and
    // a snapshot of plain lines without a converter.
    let mut note = api.note(&id)["note"].clone();
    for b in note["blocks"].as_array_mut().unwrap() {
        if b["kind"] == "content" && b["labels"] == json!(["description"]) {
            b["blocks"] =
                json!([{ "kind": "paragraph", "fields": { "text": "<b>Agenda</b><br>one" } }]);
        }
        if b["kind"] == "dict" && b["labels"] == json!(["snapshot"]) {
            let f = b["fields"].as_object_mut().unwrap();
            f.remove("converter");
            f.insert("description".into(), json!("<b>Agenda</b><br>one"));
        }
    }
    api.google.clear_requests();
    put_note(&api, &id, note);
    // The handler sees the description as the older sync wrote it: nothing to send.
    assert!(api.writes().is_empty(), "{:?}", api.writes());
    // A state from before the conversion lists every event again.
    let mut state = api.server.secret_json("google_calendar_state");
    state.as_object_mut().unwrap().remove("converter");
    expect(
        api.req("PUT", "/api/secrets/google_calendar_state")
            .json(&json!({ "value": state.to_string() }))
            .send()
            .unwrap(),
        200,
    );
    api.sync();
    let note = api.note(&id);
    assert_eq!(
        block(&note, "content", "description")["blocks"],
        json!([{ "kind": "paragraph", "fields": { "text": "**Agenda**\none" } }])
    );
    assert_in_step(&api, &note, "primary");
    assert!(api.writes().is_empty(), "{:?}", api.writes());
}
