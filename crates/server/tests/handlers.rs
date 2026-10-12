//! S1 with S3 checks: event handlers and wscript migrations (D3, D5, D6, D9, D30).
//!
//! Every type, script and starting note is seeded into the vault folder before the server starts.
//! Scripts live in `scripts/<name>.ws` and handlers are registered in the type file.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const TEMPLATE: &str = r#"
template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// An article counts the words of its title into a computed field on every update.
const ARTICLE: &str = r#"label = "Article"

field words {
  kind     = "int"
  computed = true
}

handler "before_update" {
  script   = "count"
  function = "count_words"
}
"#;

const COUNT: &str = r#"use vault

fn count_words(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    let title = fields.get("title").unwrap().as_string().unwrap()
    fields["words"] = Value::Int(title.split(" ").len())
    Ok(Value::Map(#{ "fields": Value::Map(fields) }))
}
"#;

/// A gated note refuses some titles when it is created.
const GATED: &str = r#"label = "Gated"

handler "before_create" {
  script = "gate"
}
"#;

const GATE: &str = r#"fn before_create(note: Value) -> Result[Value, string] {
    let title = note.get("note").unwrap().get("fields").unwrap().get("title").unwrap().as_string().unwrap()
    if title.starts_with("forbidden") {
        Err("a title may not start with \"forbidden\"")
    } else {
        Ok(Value::Null)
    }
}
"#;

/// A report is marked as reviewed by a handler after every update.
const REPORT: &str = r#"label = "Report"

field review {
  kind = "string"
}

handler "after_update" {
  script = "review"
}
"#;

const REVIEW: &str = r#"use vault

fn after_update(note: Value) -> Result[Value, string] {
    let id = note.get("id").unwrap().as_string().unwrap()
    let version = note.get("version").unwrap().as_string().unwrap()
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    if fields.contains_key("review") {
        Ok(Value::Null)
    } else {
        fields["review"] = Value::String("seen")
        vault::update(id, Value::Map(#{ "fields": Value::Map(fields) }), version)
    }
}
"#;

/// `ping` and `pong` notes point at each other, and each type's handler bumps the count of the
/// note its note points at: two handlers updating each other's notes.
fn bouncer(label: &str, other: &str, event: &str, script: &str) -> String {
    format!(
        r#"label = "{label}"

field partner {{
  kind   = "reference"
  target = "{other}"
}}

field count {{
  kind = "int"
}}

handler "{event}" {{
  script   = "{script}"
  function = "bump"
}}
"#
    )
}

const BUMP: &str = r#"use vault

fn bump(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    match fields.get("partner") {
        None => Ok(Value::Null),
        Some(p) => {
            let partner = p.as_string().unwrap()
            let other = vault::get(partner)?
            let version = other.get("version").unwrap().as_string().unwrap()
            let theirs = other.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
            let n = theirs.get("count").unwrap().as_int().unwrap()
            theirs["count"] = Value::Int(n + 1)
            vault::update(partner, Value::Map(#{ "fields": Value::Map(theirs) }), version)?
            Ok(Value::Null)
        }
    }
}
"#;

const PING: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const PONG: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const TICK: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";
const TOCK: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZD";

const PERSON: &str = "label = \"Person\"\n";

/// A meeting loses an attendee who is deleted, and its handlers notice.
const MEETING: &str = r#"label = "Meeting"

field attendee {
  kind   = "reference"
  target = "person"
}

field status {
  kind = "string"
}

field audited {
  kind = "string"
}

handler "before_update" {
  script = "attendance"
}

handler "after_update" {
  script = "audit"
}
"#;

const ATTENDANCE: &str = r#"fn before_update(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    if fields.contains_key("attendee") {
        Ok(Value::Null)
    } else {
        fields["status"] = Value::String("attendee removed")
        Ok(Value::Map(#{ "fields": Value::Map(fields) }))
    }
}
"#;

const AUDIT: &str = r#"use vault

fn after_update(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    if fields.contains_key("audited") {
        Ok(Value::Null)
    } else {
        fields["audited"] = Value::String("yes")
        let id = note.get("id").unwrap().as_string().unwrap()
        let version = note.get("version").unwrap().as_string().unwrap()
        vault::update(id, Value::Map(#{ "fields": Value::Map(fields) }), version)
    }
}
"#;

/// An item's estimate is a whole number of hours, until a migration makes it a string.
const ITEM: &str = r#"label = "Item"

field estimate {
  kind = "int"
}
"#;

const HOURS: &str = r#"fn to_text(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    match fields.get("estimate") {
        None => Ok(Value::Null),
        Some(v) => {
            let n = v.as_int().unwrap()
            if n == 13 {
                Err("13 is unlucky")
            } else {
                fields["estimate"] = Value::String(fmt("{}h", n))
                Ok(Value::Map(#{ "fields": Value::Map(fields) }))
            }
        }
    }
}
"#;

fn note_file(title: &str, extra: &str) -> String {
    format!(
        "title   = \"{title}\"\ncreated = \"2026-01-01T00:00:00Z\"\nupdated = \"2026-01-01T00:00:00Z\"\n{extra}"
    )
}

fn seed(vault: &Path) {
    let write = |path: &str, body: &str| {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    let types = [
        ("article", ARTICLE.to_owned()),
        ("gated", GATED.to_owned()),
        ("report", REPORT.to_owned()),
        ("ping", bouncer("Ping", "pong", "before_update", "ping")),
        ("pong", bouncer("Pong", "ping", "before_update", "pong")),
        ("tick", bouncer("Tick", "tock", "after_update", "tick")),
        ("tock", bouncer("Tock", "tick", "after_update", "tock")),
        ("person", PERSON.to_owned()),
        ("meeting", MEETING.to_owned()),
        ("item", ITEM.to_owned()),
    ];
    for (name, body) in types {
        write(&format!("types/{name}.wcl"), &format!("{body}{TEMPLATE}"));
    }
    for (name, body) in [
        ("count", COUNT),
        ("gate", GATE),
        ("review", REVIEW),
        ("ping", BUMP),
        ("pong", BUMP),
        ("tick", BUMP),
        ("tock", BUMP),
        ("attendance", ATTENDANCE),
        ("audit", AUDIT),
        ("hours", HOURS),
    ] {
        write(&format!("scripts/{name}.ws"), body);
    }
    for (type_name, id, title, partner) in [
        ("ping", PING, "Ping", PONG),
        ("pong", PONG, "Pong", PING),
        ("tick", TICK, "Tick", TOCK),
        ("tock", TOCK, "Tock", TICK),
    ] {
        write(
            &format!("notes/{type_name}/{id}.wcl"),
            &note_file(title, &format!("partner = \"{partner}\"\ncount   = 0\n")),
        );
    }
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(seed);
        let mut auth = HeaderMap::new();
        auth.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", server.admin_token())).unwrap(),
        );
        let client = Client::builder().default_headers(auth).build().unwrap();
        Api { server, client }
    }

    fn post_note(&self, type_name: &str, fields: Value) -> Response {
        self.client
            .post(self.server.url("/api/notes"))
            .json(&json!({ "type": type_name, "note": { "fields": fields } }))
            .send()
            .unwrap()
    }

    fn create(&self, type_name: &str, fields: Value) -> Value {
        expect(self.post_note(type_name, fields), 201)
    }

    fn put_note(&self, id: &str, base: &str, fields: Value) -> Response {
        self.client
            .put(self.server.url(&format!("/api/notes/{id}")))
            .json(&json!({ "base_version": base, "note": { "fields": fields } }))
            .send()
            .unwrap()
    }

    fn update(&self, id: &str, fields: Value) -> Value {
        let base = self.version(id);
        expect(self.put_note(id, &base, fields), 200)
    }

    fn delete(&self, id: &str) -> Value {
        let base = self.version(id);
        let res = self
            .client
            .delete(
                self.server
                    .url(&format!("/api/notes/{id}?base_version={base}")),
            )
            .send()
            .unwrap();
        expect(res, 200)
    }

    fn read(&self, id: &str) -> Value {
        let res = self
            .client
            .get(self.server.url(&format!("/api/notes/{id}")))
            .send()
            .unwrap();
        expect(res, 200)
    }

    fn version(&self, id: &str) -> String {
        s(&self.read(id)["version"]).to_owned()
    }

    fn fields(&self, id: &str) -> Value {
        self.read(id)["note"]["fields"].clone()
    }

    fn history(&self, id: &str) -> Vec<Value> {
        let res = self
            .client
            .get(self.server.url(&format!("/api/notes/{id}/history")))
            .send()
            .unwrap();
        expect(res, 200)["versions"].as_array().unwrap().clone()
    }

    fn change_type(&self, name: &str, body: Value) -> Response {
        self.client
            .post(self.server.url(&format!("/api/types/{name}/change")))
            .json(&body)
            .send()
            .unwrap()
    }

    fn file(&self, type_name: &str, id: &str) -> Option<String> {
        let path = self
            .server
            .vault_path()
            .join(format!("notes/{type_name}/{id}.wcl"));
        std::fs::read_to_string(path).ok()
    }

    fn files(&self, type_name: &str) -> Vec<String> {
        let dir = self.server.vault_path().join(format!("notes/{type_name}"));
        match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "unexpected status, body: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

/// Wait until `done` holds, polling, or fail after a few seconds.
fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(50));
    }
}

fn handler_authored(v: &Value, name: &str) -> bool {
    v["author"] == json!({ "kind": "handler", "name": name })
}

#[test]
fn a_before_update_handler_computes_a_field_that_clients_cannot_write() {
    let api = Api::start();
    let note = api.create("article", json!({ "title": "Hello" }));
    let id = s(&note["id"]);
    // The handler runs on update only, so the new note has no count yet.
    assert!(note["note"]["fields"].get("words").is_none(), "{note}");

    let saved = api.update(id, json!({ "title": "Hello brave new world" }));
    assert_eq!(saved["note"]["fields"]["words"], json!(4));
    let file = api.file("article", id).unwrap();
    assert!(file.contains("words   = 4"), "{file}");

    // A client write to the computed field is refused, and the file keeps the handler's value.
    let base = api.version(id);
    let res = api.put_note(id, &base, json!({ "title": "Hello", "words": 99 }));
    let body = expect(res, 400);
    assert_eq!(body["error"], "invalid");
    assert_eq!(body["issues"][0]["at"], "words");
    assert!(
        s(&body["issues"][0]["message"]).contains("computed"),
        "{body}"
    );
    assert_eq!(api.file("article", id).unwrap(), file);

    // Sending the value the note already holds is not a write to it, and the handler still runs.
    let saved = api.update(id, json!({ "title": "Two words", "words": 4 }));
    assert_eq!(saved["note"]["fields"]["words"], json!(2));
}

#[test]
fn a_before_create_handler_rejects_a_save_with_its_message() {
    let api = Api::start();
    let res = api.post_note("gated", json!({ "title": "forbidden fruit" }));
    let body = expect(res, 422);
    assert_eq!(body["error"], "rejected");
    assert_eq!(body["message"], "a title may not start with \"forbidden\"");
    assert!(api.files("gated").is_empty(), "{:?}", api.files("gated"));

    let ok = api.create("gated", json!({ "title": "allowed" }));
    assert_eq!(api.files("gated"), [format!("{}.wcl", s(&ok["id"]))]);
}

#[test]
fn an_after_update_handler_runs_after_the_response_as_the_handler() {
    let api = Api::start();
    let note = api.create("report", json!({ "title": "Q3" }));
    let id = s(&note["id"]).to_owned();

    let saved = api.update(&id, json!({ "title": "Q3 final" }));
    // The response is the client's own write: the handler has not changed it.
    assert!(saved["note"]["fields"].get("review").is_none(), "{saved}");
    let client_version = s(&saved["version"]).to_owned();

    eventually("the after_update handler's edit", || {
        api.history(&id).len() == 3
    });
    let history = api.history(&id);
    assert_eq!(history[1]["version"], json!(client_version));
    assert!(!handler_authored(&history[1], "review"), "{history:?}");
    assert!(handler_authored(&history[2], "review"), "{history:?}");
    assert_eq!(history[2]["current"], json!(true));
    assert_eq!(api.fields(&id)["review"], "seen");

    // The handler's own update runs the handler again, which finds nothing left to do.
    thread::sleep(Duration::from_millis(500));
    assert_eq!(api.history(&id).len(), 3);
}

#[test]
fn before_handlers_updating_each_others_notes_stop_at_the_depth_limit() {
    let api = Api::start();
    let ping_file = api.file("ping", PING).unwrap();
    let pong_file = api.file("pong", PONG).unwrap();

    let base = api.version(PING);
    let res = api.put_note(
        PING,
        &base,
        json!({ "title": "Ping again", "partner": PONG, "count": 0 }),
    );
    let body = expect(res, 422);
    let message = s(&body["message"]);
    assert!(message.contains("depth limit"), "{message}");
    assert!(message.contains("cycle"), "{message}");
    assert!(
        message.contains(&format!("ping {PING} before_update"))
            && message.contains(&format!("pong {PONG} before_update")),
        "{message}"
    );

    // Nothing was written.
    assert_eq!(api.file("ping", PING).unwrap(), ping_file);
    assert_eq!(api.file("pong", PONG).unwrap(), pong_file);
    assert_eq!(api.history(PING).len(), 1);
    assert_eq!(api.history(PONG).len(), 1);
}

#[test]
fn after_handlers_updating_each_others_notes_stop_at_the_depth_limit() {
    let api = Api::start();
    api.update(
        TICK,
        json!({ "title": "Tick again", "partner": TOCK, "count": 0 }),
    );

    // The chain runs from the queue, one note then the other, and then stops.
    let total = || api.history(TICK).len() + api.history(TOCK).len();
    eventually("the chain to start", || total() > 4);
    let mut last = total();
    loop {
        thread::sleep(Duration::from_millis(750));
        let now = total();
        if now == last {
            break;
        }
        last = now;
    }
    assert!(last < 20, "the chain ran on: {last} versions");
}

#[test]
fn deleting_a_note_runs_the_update_handlers_of_its_unlinked_referrers() {
    let api = Api::start();
    let person = api.create("person", json!({ "title": "Ada" }));
    let pid = s(&person["id"]).to_owned();
    let meeting = api.create("meeting", json!({ "title": "Sync", "attendee": pid }));
    let mid = s(&meeting["id"]).to_owned();
    assert!(api.fields(&mid).get("status").is_none());

    let deleted = api.delete(&pid);
    assert_eq!(deleted["also"][0]["id"], json!(mid));

    // The before_update handler ran inside the delete's write.
    let fields = api.fields(&mid);
    assert!(fields.get("attendee").is_none(), "{fields}");
    assert_eq!(fields["status"], "attendee removed");

    // The after_update handler ran from the queue.
    eventually("the meeting's after_update handler", || {
        api.history(&mid)
            .iter()
            .any(|v| handler_authored(v, "audit"))
    });
    assert_eq!(api.fields(&mid)["audited"], "yes");
}

#[test]
fn a_migration_transforms_every_note_of_a_type_in_one_commit() {
    let api = Api::start();
    let mut by_hours: Vec<(String, &str)> = [(2, "2h"), (3, "3h"), (5, "5h")]
        .iter()
        .map(|(n, h)| {
            let note = api.create("item", json!({ "title": "Item", "estimate": n }));
            (s(&note["id"]).to_owned(), *h)
        })
        .collect();
    // The plan lists notes in id order.
    by_hours.sort();
    let ids: Vec<String> = by_hours.iter().map(|(id, _)| id.clone()).collect();
    let unlucky =
        s(&api.create("item", json!({ "title": "Unlucky", "estimate": 13 }))["id"]).to_owned();
    let new_type = format!(
        "{}{TEMPLATE}",
        ITEM.replace("kind = \"int\"", "kind = \"string\"")
    );
    let before: Vec<String> = ids.iter().map(|id| api.file("item", id).unwrap()).collect();

    // Without a migration the change leaves invalid notes, so it is refused.
    let res = api.change_type("item", json!({ "wcl": new_type }));
    assert_eq!(expect(res, 400)["error"], "invalid_type");

    // One note the migration fails on fails the whole change, and nothing is written.
    let migration = json!({ "script": "hours", "function": "to_text" });
    let res = api.change_type("item", json!({ "wcl": new_type, "migration": migration }));
    let body = expect(res, 422);
    assert_eq!(body["error"], "migration_failed");
    assert_eq!(body["id"], json!(unlucky));
    assert!(s(&body["message"]).contains("13 is unlucky"), "{body}");
    api.delete(&unlucky);

    // The plan lists every note the migration changes, and changes none of them.
    let res = api.change_type("item", json!({ "wcl": new_type, "migration": migration }));
    let plan = expect(res, 200);
    assert_eq!(plan["applied"], false);
    let planned: Vec<&str> = plan["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| s(&c["id"]))
        .collect();
    assert_eq!(planned, ids);
    assert_eq!(
        plan["changes"][0]["edits"],
        json!([{ "op": "migrate", "script": "hours", "function": "to_text" }])
    );
    let unchanged: Vec<String> = ids.iter().map(|id| api.file("item", id).unwrap()).collect();
    assert_eq!(unchanged, before);

    let res = api.change_type(
        "item",
        json!({ "wcl": new_type, "migration": migration, "plan": plan["plan"] }),
    );
    let applied = expect(res, 200);
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["notes"].as_array().unwrap().len(), 3);
    for (id, hours) in &by_hours {
        let file = api.file("item", id).unwrap();
        assert!(file.contains(&format!("estimate = \"{hours}\"")), "{file}");
        assert_eq!(api.history(id).len(), 2);
    }
    let ty = api
        .client
        .get(api.server.url("/api/types/item"))
        .send()
        .unwrap();
    assert!(s(&expect(ty, 200)["wcl"]).contains("kind = \"string\""));
}
