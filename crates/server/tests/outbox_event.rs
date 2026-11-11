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
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, FakeGoogle, TestAccount, TestServer};

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
        let outbox_type = api.file("types/outbox.wcl");
        expect(
            api.req("PUT", "/api/types/outbox")
                .body(outbox_type.replace("allow_domains = [", &allow))
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

    fn draft_event(&self, fake: &FakeAnthropic, event: &str, changes: Value) -> String {
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model("claude-opus-5"))
            .secrets()
        {
            expect(
                self.req("PUT", &format!("/api/secrets/{name}"))
                    .json(&json!({ "value": value }))
                    .send()
                    .unwrap(),
                200,
            );
        }
        expect(self.req("PUT", "/api/agents/clerk").header("content-type", "text/plain").body("label = \"Clerk\"\npermissions {\n tools = [\"draft_event\"]\n confirm {\n notes = false\n }\n}\n").send().unwrap(), 200);
        let mut input = json!({ "event": event, "event_action": "", "my_response": "", "title": "", "when": { "start": "", "end": "" }, "location": "", "attendees": [], "description": "", "calendar": "" });
        for (key, value) in changes.as_object().unwrap() {
            input[key] = value.clone();
        }
        fake.push(AnthropicMessage::new().tool_use("draft", "draft_event", input));
        fake.push(AnthropicMessage::text("Drafted."));
        let response = self.req("POST", "/api/ai/chat").json(&json!({ "agent": "clerk", "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Draft this action for review." }] }] })).send().unwrap();
        assert_eq!(response.status(), 200);
        let stream = response.text().unwrap();
        let drafts = expect(
            self.req("POST", "/api/query")
                .json(&json!({ "query": "from outbox where status == \"draft\"" }))
                .send()
                .unwrap(),
            200,
        );
        let rows = drafts["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{stream}\n{drafts}");
        let id = rows[0]["id"].as_str().unwrap().to_owned();
        assert_eq!(self.fields(&id)["drafted_by"], "clerk");
        id
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

    fn assert_trashed(&self, id: &str) {
        assert_eq!(self.req("GET", &format!("/api/notes/{id}")).send().unwrap().status(), 404);
        let trash = expect(self.req("GET", "/api/trash").send().unwrap(), 200);
        assert!(trash["notes"].as_array().unwrap().iter().any(|n| n["id"] == id), "{trash}");
    }

    fn delete_item(&self, id: &str) -> String {
        self.create("outbox", invite(json!({ "title": "Delete standup", "event": id, "event_action": "delete" }), None))
    }

    fn retry_delete(&self, item: &str, id: &str) {
        self.edit(item, json!({ "status": "ready" }));
        assert!(self.gone(item));
        self.assert_trashed(id);
        assert_eq!(self.google.event("primary", "standup").unwrap()["status"], "cancelled");
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
fn agent_cannot_bypass_event_outbox_with_send_updates() {
    let root = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(root.path()).unwrap();
    let state = state::State::open(
        &vault.server_dir().join("state.db"),
        Some(state::SecretKey::generate()),
    )
    .unwrap();
    let writer = writer::Writer::open(vault, state).unwrap();
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let result = writer.create(
        "event",
        &writer::Input::Json(json!({ "fields": {
            "title": "Unreviewed invitation",
            "when": { "start": "2026-04-03", "end": "2026-04-03" },
            "attendees_raw": ["ann@example.com"],
            "send_updates": "all"
        }})),
        &state::Author::Agent("careless".into()),
    );
    assert!(
        matches!(result, Err(writer::Error::Rejected(ref message)) if message.contains("send_updates")),
        "agent bypassed the event Outbox: {result:?}"
    );
}

#[test]
fn writer_enforces_calendar_author_and_outbox_boundary() {
    let root = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(root.path()).unwrap();
    let state = state::State::open(
        &vault.server_dir().join("state.db"),
        Some(state::SecretKey::generate()),
    )
    .unwrap();
    let w = writer::Writer::open(vault, state).unwrap();
    w.upgrade_shipped_defaults(writer::Upkeep::Home).unwrap();
    let user = state::Author::Session("owner".into());
    let agent = state::Author::Agent("careless".into());
    let sender = state::Author::Handler("outbox_send".into());
    let body = json!({ "fields": {
        "title": "Standup", "google_ref": "standup", "calendar": "primary",
        "when": { "start": "2026-04-03", "end": "2026-04-03" },
        "attendees_raw": ["ann@example.com"], "my_response": "needs_action"
    }});
    let made = w
        .create("event", &writer::Input::Json(body.clone()), &user)
        .unwrap();
    for (field, value) in [
        ("attendees_raw", json!(["bob@example.com"])),
        ("attendees", json!([])),
        ("my_response", json!("accepted")),
        ("status", json!("cancelled")),
        ("etag", json!("forged")),
        ("google_ref", json!("another-event")),
        ("send_updates", json!("all")),
    ] {
        let mut changed = body.clone();
        changed["fields"][field] = value;
        let result = w.update(
            &made.id,
            &writer::Input::Json(changed),
            Some(&made.version),
            &agent,
        );
        assert!(
            matches!(result, Err(writer::Error::Rejected(_))),
            "{field}: {result:?}"
        );
        assert_eq!(w.read(&made.id).unwrap().version, made.version);
    }
    assert!(matches!(
        w.delete(&made.id, Some(&made.version), &agent),
        Err(writer::Error::Rejected(_))
    ));
    let mut forged_snapshot = body.clone();
    forged_snapshot["blocks"] = json!([{ "kind": "dict", "labels": ["snapshot"], "fields": { "my_response": "declined" } }]);
    assert!(matches!(
        w.update(
            &made.id,
            &writer::Input::Json(forged_snapshot),
            Some(&made.version),
            &agent
        ),
        Err(writer::Error::Rejected(_))
    ));
    for author in [
        user.clone(),
        state::Author::Token("api".into()),
        state::Author::Task("outbox_send".into()),
        state::Author::Handler("other".into()),
    ] {
        let mut changed = body.clone();
        changed["fields"]["send_updates"] = json!("all");
        assert!(
            matches!(
                w.update(
                    &made.id,
                    &writer::Input::Json(changed),
                    Some(&made.version),
                    &author
                ),
                Err(writer::Error::Rejected(_))
            ),
            "{author:?}"
        );
    }
    let mut harmless = body.clone();
    harmless["fields"]["location"] = json!("Room 2");
    let edited = w
        .update(
            &made.id,
            &writer::Input::Json(harmless),
            Some(&made.version),
            &agent,
        )
        .unwrap();
    let mut approved = body;
    approved["fields"]["send_updates"] = json!("all");
    let approved = w
        .update(
            &made.id,
            &writer::Input::Json(approved),
            Some(&edited.version),
            &sender,
        )
        .unwrap();
    let mut agent_clear = writer::doc_to_json(&w.read(&made.id).unwrap().doc);
    agent_clear["fields"]
        .as_object_mut()
        .unwrap()
        .remove("send_updates");
    assert!(matches!(
        w.update(
            &made.id,
            &writer::Input::Json(agent_clear),
            Some(&approved.version),
            &agent
        ),
        Err(writer::Error::Rejected(_))
    ));
    // A pending approved send must not be reused by an agent's otherwise harmless edit.
    let note = w.read(&made.id).unwrap();
    let mut retry = writer::doc_to_json(&note.doc);
    retry["fields"]["location"] = json!("Room 3");
    assert!(matches!(
        w.update(
            &made.id,
            &writer::Input::Json(retry),
            Some(&approved.version),
            &agent
        ),
        Err(writer::Error::Rejected(_))
    ));
    let mut cleared = writer::doc_to_json(&w.read(&made.id).unwrap().doc);
    cleared["fields"]
        .as_object_mut()
        .unwrap()
        .remove("send_updates");
    w.update(
        &made.id,
        &writer::Input::Json(cleared),
        Some(&approved.version),
        &state::Author::Handler("google_event".into()),
    )
    .unwrap();
    w.put_agent(
        "careless",
        "label = \"Careless\"\npermissions {\n confirm {\n notes = false\n }\n}\n",
        |_| Vec::new(),
    )
    .unwrap();
    for action in [
        json!({ "my_response": "accepted" }),
        json!({ "event_action": "delete" }),
    ] {
        let mut fields =
            json!({ "title": "Review calendar action", "kind": "event", "event": made.id });
        for (key, value) in action.as_object().unwrap() {
            fields[key] = value.clone();
        }
        let draft = w
            .create(
                "outbox",
                &writer::Input::Json(json!({ "fields": fields })),
                &agent,
            )
            .unwrap();
        let mut body = writer::doc_to_json(&w.read(&draft.id).unwrap().doc);
        assert_eq!(body["fields"]["status"], "draft");
        body["fields"]["status"] = json!("ready");
        assert!(
            w.update(
                &draft.id,
                &writer::Input::Json(body),
                Some(&draft.version),
                &agent
            )
            .is_err()
        );
    }
}

#[test]
fn outbox_delete_disconnected_keeps_event_for_retry() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.events().keys().next().unwrap().clone();
    expect(
        api.req("DELETE", "/api/secrets/google_refresh_token")
            .send()
            .unwrap(),
        200,
    );
    let item = api.create(
        "outbox",
        invite(
            json!({ "title": "Delete standup", "event": id, "event_action": "delete" }),
            None,
        ),
    );
    api.server.settle_handlers();
    assert_eq!(api.fields(&item)["status"], "failed");
    assert_ne!(api.google.event("primary", "standup").unwrap()["status"], "cancelled");
    assert_eq!(api.req("GET", &format!("/api/notes/{id}")).send().unwrap().status(), 200);
    assert!(api.fields(&item)["error"].as_str().unwrap().contains("not connected"));
    expect(api.req("PUT", "/api/secrets/google_refresh_token").json(&json!({ "value": DEFAULT_REFRESH_TOKEN })).send().unwrap(), 200);
    api.retry_delete(&item, &id);
}

#[test]
fn outbox_delete_google_error_keeps_event_and_manual_retry_succeeds() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.events().keys().next().unwrap().clone();
    api.google.fail_calendar_writes(Some((500, "Backend unavailable")));
    let item = api.delete_item(&id);
    api.settle();
    assert_eq!(api.fields(&item)["status"], "failed");
    assert!(api.fields(&item)["error"].as_str().unwrap().contains("Backend unavailable"));
    assert_eq!(api.fields(&id)["google_ref"], "standup");
    assert_ne!(api.google.event("primary", "standup").unwrap()["status"], "cancelled");
    api.google.fail_calendar_writes(None);
    api.retry_delete(&item, &id);
    let deletes: Vec<_> = api.google.requests().into_iter().filter(|r| r.method == "DELETE").collect();
    assert_eq!(deletes.len(), 2);
    assert!(deletes.iter().all(|r| r.query("sendUpdates").as_deref() == Some("all")));
}

#[test]
fn outbox_delete_no_network_keeps_event_and_manual_retry_succeeds() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.events().keys().next().unwrap().clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let mut endpoints = api.google.endpoints();
    endpoints["calendar"] = json!(format!("http://{addr}/calendar/v3"));
    expect(api.req("PUT", "/api/secrets/google_settings").json(&json!({ "value": json!({ "endpoints": endpoints }).to_string() })).send().unwrap(), 200);
    let item = api.delete_item(&id);
    api.settle();
    assert_eq!(api.fields(&item)["status"], "failed");
    assert!(!api.fields(&item)["error"].as_str().unwrap().is_empty());
    assert_eq!(api.fields(&id)["google_ref"], "standup");
    assert_ne!(api.google.event("primary", "standup").unwrap()["status"], "cancelled");
    expect(api.req("PUT", "/api/secrets/google_settings").json(&json!({ "value": json!({ "endpoints": api.google.endpoints() }).to_string() })).send().unwrap(), 200);
    api.retry_delete(&item, &id);
}

#[test]
fn outbox_delete_timeout_keeps_event_and_manual_retry_succeeds() {
    let api = Api::start();
    api.google.put_event("primary", standup());
    api.sync();
    let id = api.events().keys().next().unwrap().clone();
    api.google.set_calendar_delay(Some(std::time::Duration::from_secs(360)));
    let item = api.delete_item(&id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(330);
    loop {
        if api.fields(&item)["status"] == "failed" { break; }
        assert!(std::time::Instant::now() < deadline, "delete never failed after its handler deadline");
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(!api.fields(&item)["error"].as_str().unwrap().is_empty());
    assert_eq!(api.fields(&id)["google_ref"], "standup");
    assert_ne!(api.google.event("primary", "standup").unwrap()["status"], "cancelled");
    api.google.set_calendar_delay(None);
    // The failed handler job remains as diagnostic evidence, not an automatic retry.
    assert!(!api.server.settle_handlers().is_empty());
    let mut note = api.note(&item);
    note["note"]["fields"]["status"] = json!("ready");
    expect(api.req("PUT", &format!("/api/notes/{item}")).json(&json!({ "note": note["note"], "base_version": note["version"] })).send().unwrap(), 200);
    api.server.settle_handlers();
    assert!(api.gone(&item));
    api.assert_trashed(&id);
}

#[test]
fn outbox_delete_already_gone_is_success() {
    for status in [404, 410] {
        let api = Api::start();
        api.google.put_event("primary", standup());
        api.sync();
        let id = api.events().keys().next().unwrap().clone();
        api.google.cancel_event("primary", "standup");
        api.google.fail_calendar_writes(Some((status, "Already gone")));
        let item = api.delete_item(&id);
        api.settle();
        assert!(api.gone(&item));
        api.assert_trashed(&id);
        assert_eq!(api.google.requests().iter().filter(|r| r.method == "DELETE").count(), 1);
    }
}

#[test]
fn reviewed_event_rsvp_and_delete_reach_google_through_outbox() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let mut event = standup();
    event["attendees"] =
        json!([{ "email": "me@example.com", "self": true, "responseStatus": "needsAction" }]);
    event["organizer"] = json!({ "email": "ann@example.com" });
    api.google.put_event("primary", event);
    api.sync();
    let events = api.events();
    let id = events.keys().next().unwrap();
    let rsvp = api.draft_event(&fake, id, json!({ "my_response": "accepted" }));
    api.settle();
    assert!(api.google.calendar_writes().is_empty());
    api.edit(&rsvp, json!({ "status": "ready" }));
    api.settle();
    let sent = api.one_write_after(0);
    assert_eq!(sent.method, "PATCH");
    assert_eq!(sent.send_updates.as_deref(), Some("all"));
    assert_eq!(sent.body["attendees"][0]["responseStatus"], "accepted");
    assert!(api.gone(&rsvp));
    let deletion = api.draft_event(&fake, id, json!({ "event_action": "delete" }));
    api.settle();
    assert!(api.google.requests().iter().all(|r| r.method != "DELETE"));
    api.edit(&deletion, json!({ "status": "ready" }));
    api.settle();
    let deletes: Vec<_> = api
        .google
        .requests()
        .into_iter()
        .filter(|r| r.method == "DELETE")
        .collect();
    assert_eq!(deletes.len(), 1, "{deletes:?}");
    assert_eq!(deletes[0].query("sendUpdates").as_deref(), Some("all"));
    assert_eq!(
        api.google.event("primary", "standup").unwrap()["status"],
        "cancelled"
    );
    assert!(api.gone(&deletion));
    api.assert_trashed(id);
}

#[test]
fn local_delete_fails_pending_changes_but_finishes_delete_items() {
    let api = Api::start();
    let id = api.create("event", json!({ "fields": { "title": "Local idea" } }));
    api.settle();
    let pending = api.create("outbox", json!({ "fields": { "title": "Pending change", "kind": "event", "status": "sending", "event": id, "result": id } }));
    api.settle();
    let deletion = api.create(
        "outbox",
        invite(
            json!({ "title": "Delete idea", "event": id, "event_action": "delete" }),
            None,
        ),
    );
    api.settle();
    assert!(api.gone(&deletion));
    let fields = api.fields(&pending);
    assert_eq!(fields["status"], "failed", "{fields}");
    assert!(fields["error"].as_str().unwrap().contains("deleted"));
    assert!(fields.get("sent_at").is_none());
    assert!(api.google.calendar_writes().is_empty());
}

#[test]
fn agent_calendar_creates_local_deletes_and_series_expansion_are_checked() {
    struct Clock;
    impl state::Clock for Clock {
        fn now_ms(&self) -> i64 {
            1_772_409_600_000
        }
    }
    let root = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(root.path()).unwrap();
    let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = writer::Writer::open(vault, state)
        .unwrap()
        .with_clock(std::sync::Arc::new(Clock));
    w.upgrade_shipped_defaults(writer::Upkeep::Home).unwrap();
    let user = state::Author::Session("owner".into());
    let agent = state::Author::Agent("careless".into());
    let invitation =
        json!({ "fields": { "title": "Unreviewed invite", "attendees_raw": ["ann@example.com"] } });
    assert!(matches!(
        w.create("event", &writer::Input::Json(invitation), &agent),
        Err(writer::Error::Rejected(_))
    ));
    let local = w
        .create(
            "event",
            &writer::Input::Json(json!({ "fields": { "title": "Local idea" } })),
            &agent,
        )
        .unwrap();
    w.delete(&local.id, Some(&local.version), &agent).unwrap();
    let series_body = json!({ "fields": { "title": "Series", "rule": ["RRULE:FREQ=DAILY;COUNT=1"], "starts": { "start": "2026-03-02", "end": "2026-03-02" } } });
    let mut google_series = series_body.clone();
    google_series["fields"]["google_ref"] = json!("master");
    google_series["fields"]["calendar"] = json!("primary");
    let google = w
        .create("event_series", &writer::Input::Json(google_series), &user)
        .unwrap();
    assert!(matches!(
        w.delete(&google.id, Some(&google.version), &agent),
        Err(writer::Error::Rejected(_))
    ));

    let series = w
        .create("event_series", &writer::Input::Json(series_body), &user)
        .unwrap();
    let occurrence = w
        .list(Some("event"))
        .unwrap()
        .into_iter()
        .find(|s| {
            writer::doc_to_json(&w.read(&s.id).unwrap().doc)["fields"]["series_ref"] == series.id
        })
        .expect("local series expanded");
    let note = w.read(&occurrence.id).unwrap();
    let mut body = writer::doc_to_json(&note.doc);
    body["fields"]["google_ref"] = json!("linked-occurrence");
    let linked = w
        .update(
            &occurrence.id,
            &writer::Input::Json(body),
            Some(&note.version),
            &user,
        )
        .unwrap();
    let note = w.read(&series.id).unwrap();
    let mut body = writer::doc_to_json(&note.doc);
    body["fields"]["rule"] = json!(["RDATE:20260303"]);
    let result = w.update(
        &series.id,
        &writer::Input::Json(body),
        Some(&note.version),
        &agent,
    );
    assert!(
        matches!(result, Err(writer::Error::Rejected(_))),
        "{result:?}"
    );
    assert_eq!(w.read(&occurrence.id).unwrap().version, linked.version);
    assert_eq!(w.read(&series.id).unwrap().version, note.version);
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
    // As the HTML Google takes (rich-import H4).
    assert_eq!(insert.body["description"], "<p>Bring the numbers</p>");
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
