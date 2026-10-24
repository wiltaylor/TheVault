//! S1 with the fake Google server: calendar invites and changes through the Outbox (outbox X5).
//! The shipped `outbox_send` handler writes the event note with the one-shot `send_updates`, the
//! `event` type's `google_event` write-back sends it with `sendUpdates=all` and finishes the
//! item, and a direct edit of an event note still sends no `sendUpdates`.

use std::collections::BTreeMap;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::fake_google::{
    CalendarWrite, DEFAULT_CLIENT_ID, DEFAULT_CLIENT_SECRET, DEFAULT_REFRESH_TOKEN,
};
use testkit::{FakeGoogle, TestServer};

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
    /// A server whose `event` handlers and `google_calendar` task reach the fake, with the
    /// three credential secrets set and `google_settings` pointing at it.
    fn start() -> Api {
        let google = FakeGoogle::start();
        let server = TestServer::start();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap();
        let api = Api {
            server,
            client,
            google,
        };
        let settings = json!({ "endpoints": api.google.endpoints() });
        for (name, value) in [
            ("google_client_id", DEFAULT_CLIENT_ID.to_owned()),
            ("google_client_secret", DEFAULT_CLIENT_SECRET.to_owned()),
            ("google_refresh_token", DEFAULT_REFRESH_TOKEN.to_owned()),
            ("google_settings", settings.to_string()),
        ] {
            expect(
                api.req("PUT", &format!("/api/secrets/{name}"))
                    .json(&json!({ "value": value }))
                    .send()
                    .unwrap(),
                200,
            );
        }
        let allow = format!("allow_domains = [\"{}\", ", api.google.host());
        let event_type = api.file("types/event.wcl");
        assert!(event_type.contains("field send_updates"), "{event_type}");
        expect(
            api.req("PUT", "/api/types/event")
                .body(event_type.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        let task = api.file("tasks/google_calendar.wcl");
        expect(
            api.req("PUT", "/api/tasks/google_calendar")
                .body(task.replace("allow_domains = [", &allow))
                .send()
                .unwrap(),
            200,
        );
        api
    }

    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.server.vault_path().join(path)).unwrap()
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
    }

    fn create(&self, type_name: &str, note: Value) -> String {
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

    fn fields(&self, id: &str) -> Value {
        self.note(id)["note"]["fields"].clone()
    }

    /// Whether note `id` is gone for good: not in the vault and not in the trash.
    fn gone(&self, id: &str) -> bool {
        let status = self
            .req("GET", &format!("/api/notes/{id}"))
            .send()
            .unwrap()
            .status()
            .as_u16();
        let trash = expect(self.req("GET", "/api/trash").send().unwrap(), 200);
        let trashed = trash["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == id);
        status == 404 && !trashed
    }

    /// Change fields of note `id` as the user would, then let its handlers finish.
    fn edit(&self, id: &str, changes: Value) {
        let mut note = self.note(id);
        for (k, v) in changes.as_object().unwrap() {
            if v.is_null() {
                note["note"]["fields"].as_object_mut().unwrap().remove(k);
            } else {
                note["note"]["fields"][k] = v.clone();
            }
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

    /// Let every queued handler run finish, and fail on any that failed.
    fn settle(&self) {
        let failed = self.server.settle_handlers();
        assert!(failed.is_empty(), "{failed:#?}");
    }

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

    /// Every event note, by id.
    fn events(&self) -> BTreeMap<String, Value> {
        let page = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": "from event include archived", "page_size": 1000 }))
                .send()
                .unwrap(),
            200,
        );
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["id"].as_str().unwrap().to_owned(), r.clone()))
            .collect()
    }

    /// The one write the fake Calendar received since `before` writes.
    fn one_write_after(&self, before: usize) -> CalendarWrite {
        let writes = self.google.calendar_writes();
        assert_eq!(writes.len(), before + 1, "{writes:#?}");
        writes.into_iter().last().unwrap()
    }
}

fn paragraph(text: &str) -> Value {
    json!({ "kind": "paragraph", "fields": { "text": text } })
}

/// An event item marked ready by the user, so it sends at once.
fn invite(fields: Value, description: Option<&str>) -> Value {
    let mut f = json!({ "kind": "event", "status": "ready" });
    for (k, v) in fields.as_object().unwrap() {
        f[k] = v.clone();
    }
    let blocks: Vec<Value> = description
        .map(|d| json!({ "kind": "content", "labels": ["description"], "blocks": [paragraph(d)] }))
        .into_iter()
        .collect();
    json!({ "fields": f, "blocks": blocks })
}

fn standup() -> Value {
    json!({
        "id": "standup",
        "summary": "Standup",
        "location": "Room 1",
        "start": { "dateTime": "2026-03-05T10:00:00Z" },
        "end": { "dateTime": "2026-03-05T10:15:00Z" },
        "attendees": [{ "email": "ann@example.com", "responseStatus": "accepted" }],
        "organizer": { "email": "me@example.com" },
    })
}

fn assert_no_send_updates(note: &Value) {
    assert!(
        note["note"]["fields"].get("send_updates").is_none(),
        "{note:#}"
    );
}

#[test]
fn a_ready_new_event_is_inserted_with_send_updates_and_the_item_leaves_the_outbox() {
    let api = Api::start();
    let item = api.create(
        "outbox",
        invite(
            json!({
                "title": "Planning",
                "when": { "start": "2026-04-02T09:00:00+11:00", "end": "2026-04-02T10:00:00+11:00" },
                "location": "Room 4",
                "attendees_raw": ["ann@example.com", "bob@example.com"],
            }),
            Some("Bring the numbers"),
        ),
    );
    api.settle();

    let insert = api.one_write_after(0);
    assert_eq!(insert.method, "POST");
    assert_eq!(insert.calendar, "primary");
    assert_eq!(insert.send_updates.as_deref(), Some("all"));
    assert_eq!(insert.status, 200);
    assert_eq!(insert.body["summary"], "Planning");
    assert_eq!(insert.body["location"], "Room 4");
    assert_eq!(insert.body["description"], "Bring the numbers");
    assert_eq!(
        insert.body["start"]["dateTime"],
        "2026-04-02T09:00:00+11:00"
    );
    assert_eq!(
        insert.body["attendees"],
        json!([{ "email": "ann@example.com" }, { "email": "bob@example.com" }])
    );

    let events = api.events();
    assert_eq!(events.len(), 1, "{events:#?}");
    let event_id = events.keys().next().unwrap().clone();
    let event = api.note(&event_id);
    let ef = &event["note"]["fields"];
    assert_eq!(ef["title"], "Planning");
    assert_eq!(ef["google_ref"], insert.id.as_str());
    assert_eq!(ef["calendar"], "primary");
    // The one-shot field is cleared by the write-back.
    assert_no_send_updates(&event);

    // Sent, the item leaves the Outbox: the event note is the regular copy.
    assert!(api.gone(&item));

    // A direct edit of the event afterwards emails no one.
    api.edit(&event_id, json!({ "location": "Room 5" }));
    let patch = api.one_write_after(1);
    assert_eq!(patch.method, "PATCH");
    assert_eq!(patch.send_updates, None);
    assert_eq!(patch.body, json!({ "location": "Room 5" }));
}

#[test]
fn a_change_item_patches_only_its_non_empty_fields_with_send_updates() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let events = api.events();
    let (event_id, _) = events.iter().next().unwrap();
    assert!(api.google.calendar_writes().is_empty());

    // A direct edit sends no sendUpdates.
    api.edit(event_id, json!({ "title": "Daily standup" }));
    let edit = api.one_write_after(0);
    assert_eq!(edit.send_updates, None);
    assert_eq!(edit.body, json!({ "summary": "Daily standup" }));

    // The change keeps the title (as the draft tools write it) and moves the time.
    let item = api.create(
        "outbox",
        invite(
            json!({
                "title": "Daily standup",
                "event": event_id,
                "when": { "start": "2026-03-05T11:00:00Z", "end": "2026-03-05T11:15:00Z" },
                "attendees_raw": ["ann@example.com", "carol@example.com"],
            }),
            None,
        ),
    );
    api.settle();

    let patch = api.one_write_after(1);
    assert_eq!(patch.method, "PATCH");
    assert_eq!(patch.id, "standup");
    assert_eq!(patch.send_updates.as_deref(), Some("all"));
    let body = patch.body.as_object().unwrap();
    let mut keys: Vec<&str> = body.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["attendees", "end", "start"], "{body:#?}");
    assert_eq!(patch.body["start"]["dateTime"], "2026-03-05T11:00:00Z");
    // Ann keeps her response; Carol is new.
    assert_eq!(
        patch.body["attendees"],
        json!([
            { "email": "ann@example.com", "responseStatus": "accepted" },
            { "email": "carol@example.com" },
        ])
    );

    let event = api.note(event_id);
    assert_eq!(event["note"]["fields"]["location"], "Room 1");
    assert_eq!(
        event["note"]["fields"]["when"]["start"],
        "2026-03-05T11:00:00Z"
    );
    assert_no_send_updates(&event);
    assert!(api.gone(&item));

    // The next direct edit emails no one again.
    api.edit(event_id, json!({ "location": "Room 2" }));
    let after = api.one_write_after(2);
    assert_eq!(after.send_updates, None);
    assert_eq!(after.body, json!({ "location": "Room 2" }));
}

#[test]
fn a_google_error_fails_the_item_with_its_message_and_a_retry_reuses_the_event() {
    let api = Api::start();
    api.google.fail_calendar_writes(Some((
        403,
        "You need to have writer access to this calendar.",
    )));
    let item = api.create(
        "outbox",
        invite(
            json!({
                "title": "Review",
                "when": { "start": "2026-04-03", "end": "2026-04-03" },
                "attendees_raw": ["ann@example.com"],
            }),
            None,
        ),
    );
    api.settle();

    let refused = api.one_write_after(0);
    assert_eq!(refused.status, 403);
    assert_eq!(refused.send_updates.as_deref(), Some("all"));
    let f = api.fields(&item);
    assert_eq!(f["status"], "failed", "{f:#}");
    let error = f["error"].as_str().unwrap();
    assert!(
        error.contains("You need to have writer access to this calendar."),
        "{error}"
    );
    let events = api.events();
    assert_eq!(events.len(), 1, "{events:#?}");
    let event_id = events.keys().next().unwrap().clone();
    assert_eq!(f["result"], event_id.as_str());
    // Left set, so the retry sends it again.
    assert_eq!(api.fields(&event_id)["send_updates"], "all");

    // The user retries once Google takes writes again.
    api.google.fail_calendar_writes(None);
    api.edit(&item, json!({ "status": "ready", "error": null }));

    let insert = api.one_write_after(1);
    assert_eq!(insert.method, "POST");
    assert_eq!(insert.status, 200);
    assert_eq!(insert.send_updates.as_deref(), Some("all"));
    assert_eq!(api.events().len(), 1);
    let event = api.note(&event_id);
    assert_eq!(event["note"]["fields"]["google_ref"], insert.id.as_str());
    assert_no_send_updates(&event);
    assert!(api.gone(&item));
}

#[test]
fn a_change_that_changes_nothing_fails_and_clears_send_updates() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let events = api.events();
    let (event_id, _) = events.iter().next().unwrap();

    let item = api.create(
        "outbox",
        invite(
            json!({ "title": "Standup", "event": event_id, "location": "Room 1" }),
            None,
        ),
    );
    api.settle();

    assert!(
        api.google.calendar_writes().is_empty(),
        "{:#?}",
        api.google.calendar_writes()
    );
    let f = api.fields(&item);
    assert_eq!(f["status"], "failed", "{f:#}");
    assert!(
        f["error"].as_str().unwrap().contains("nothing to send"),
        "{f:#}"
    );
    assert_no_send_updates(&api.note(event_id));
}

#[test]
fn a_change_cannot_move_an_event_to_another_calendar() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let events = api.events();
    let (event_id, _) = events.iter().next().unwrap();

    let item = api.create(
        "outbox",
        invite(
            json!({ "title": "Standup", "event": event_id, "calendar": "work" }),
            None,
        ),
    );
    api.settle();

    let f = api.fields(&item);
    assert_eq!(f["status"], "failed", "{f:#}");
    assert!(
        f["error"].as_str().unwrap().contains("another calendar"),
        "{f:#}"
    );
    assert!(api.google.calendar_writes().is_empty());
    assert_no_send_updates(&api.note(event_id));
}
