//! S1 with the fake Anthropic: the activity log (activity Y1 to Y3). Each source records its
//! entry with the right kind, category, actor, summary and subject; no message text or secret is
//! copied; `GET /api/activity` needs `admin` and pages; new entries arrive as the `activity` live
//! event, to `admin` callers only.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, PASSWORD, TestAccount, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MODEL: &str = "claude-opus-5";

const MEMO: &str = r#"label = "Memo"

field status {
  kind = "string"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
    slot "body"
  }
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_with_args(
            |vault| {
                std::fs::create_dir_all(vault.join("types")).unwrap();
                std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
            },
            &[OsStr::new("--test-clock"), OsStr::new(START)],
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Api { server, client }
    }

    fn req_as(&self, token: &str, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .bearer_auth(token)
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.req_as(self.server.admin_token(), method, path)
    }

    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let res = self.req(method, path).json(&body).send().unwrap();
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        expect(self.req("GET", path).send().unwrap(), 200)
    }

    fn token(&self, name: &str, scope: &str) -> String {
        let made = self.send(
            "POST",
            "/api/tokens",
            json!({ "name": name, "scope": scope }),
        );
        made["secret"].as_str().unwrap().to_owned()
    }

    /// A password session's cookie.
    fn login(&self) -> String {
        let res = self
            .client
            .post(self.server.url("/api/login"))
            .header("user-agent", "activity-test")
            .json(&json!({ "password": PASSWORD }))
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
        res.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    fn anthropic(&self, fake: &FakeAnthropic) {
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            self.send(
                "PUT",
                &format!("/api/secrets/{name}"),
                json!({ "value": value }),
            );
        }
    }

    fn memo(&self, title: &str, status: &str) -> (String, String) {
        let v = self.send(
            "POST",
            "/api/notes?type=memo",
            json!({ "note": { "fields": { "title": title, "status": status } } }),
        );
        (
            v["id"].as_str().unwrap().to_owned(),
            v["version"].as_str().unwrap().to_owned(),
        )
    }

    /// Every entry, oldest first, of `query` (such as `category=ai`), read a page at a time.
    fn entries(&self, query: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let mut before: Option<String> = None;
        loop {
            let mut path = format!("/api/activity?limit=200&{query}");
            if let Some(b) = &before {
                path.push_str(&format!("&before={b}"));
            }
            let page = self.get(&path);
            out.extend(page["items"].as_array().unwrap().iter().cloned());
            match page["next"].as_str() {
                Some(n) => before = Some(n.to_owned()),
                None => break,
            }
        }
        out.reverse();
        out
    }

    /// Move the test clock on, which runs whatever is due and waits for it.
    fn advance(&self, by: &str) {
        self.send("POST", "/api/test/clock", json!({ "advance": by }));
    }

    /// One chat turn, read to the end of its stream.
    fn chat(&self, body: Value) -> Vec<(String, Value)> {
        let res = self.req("POST", "/api/ai/chat").json(&body).send().unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let mut out = Vec::new();
        let mut name = String::new();
        let mut data = String::new();
        for line in BufReader::new(res).lines() {
            let Ok(line) = line else { break };
            if line.is_empty() {
                if !data.is_empty() {
                    out.push((
                        std::mem::take(&mut name),
                        serde_json::from_str(&data).unwrap(),
                    ));
                    data.clear();
                }
                continue;
            }
            if let Some(v) = line.strip_prefix("event:") {
                name = v.trim().to_owned();
            } else if let Some(v) = line.strip_prefix("data:") {
                data.push_str(v.strip_prefix(' ').unwrap_or(v));
            }
        }
        out
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn kinds(entries: &[Value]) -> Vec<&str> {
    entries
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect()
}

fn actor(e: &Value) -> String {
    format!(
        "{}:{}",
        e["actor_kind"].as_str().unwrap(),
        e["actor"].as_str().unwrap()
    )
}

fn find<'a>(entries: &'a [Value], kind: &str) -> &'a Value {
    entries
        .iter()
        .find(|e| e["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} in {entries:#?}"))
}

#[test]
fn note_edits_type_changes_and_sign_ins_are_recorded_by_their_actor() {
    let api = Api::start();
    // A failed sign-in, then one that works.
    let res = api
        .client
        .post(api.server.url("/api/login"))
        .json(&json!({ "password": "not the password" }))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);
    let cookie = api.login();
    let session = |method: &str, path: &str| {
        api.client
            .request(method.parse().unwrap(), api.server.url(path))
            .header("cookie", &cookie)
    };

    // The user creates a note and edits it twice within a minute: one update entry.
    let made = expect(
        session("POST", "/api/notes?type=memo")
            .json(&json!({ "note": { "fields": { "title": "Roof", "status": "open" } } }))
            .send()
            .unwrap(),
        201,
    );
    let id = made["id"].as_str().unwrap().to_owned();
    let mut version = made["version"].as_str().unwrap().to_owned();
    for title in ["Roof plan", "Roof plan B"] {
        let saved = expect(
            session("PUT", &format!("/api/notes/{id}"))
                .json(&json!({
                    "note": { "fields": { "title": title, "status": "open" } },
                    "base_version": version,
                }))
                .send()
                .unwrap(),
            200,
        );
        version = saved["version"].as_str().unwrap().to_owned();
    }
    // A token's edit is an entry of its own.
    api.send(
        "PUT",
        &format!("/api/notes/{id}"),
        json!({
            "note": { "fields": { "title": "Roof plan C", "status": "open" } },
            "base_version": version,
        }),
    );

    // A type made and changed with the admin token.
    let ty =
        "label = \"Idea\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";
    expect(
        api.req("PUT", "/api/types/idea")
            .header("content-type", "text/plain")
            .body(ty)
            .send()
            .unwrap(),
        200,
    );
    expect(
        api.req("PUT", "/api/types/idea")
            .header("content-type", "text/plain")
            .body(ty.replace("Idea", "Big idea"))
            .send()
            .unwrap(),
        200,
    );

    let notes = api.entries("category=notes");
    let ours: Vec<&Value> = notes
        .iter()
        .filter(|e| e["subject"] == id.as_str())
        .collect();
    let got: Vec<(String, String, String)> = ours
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap().to_owned(),
                actor(e),
                e["summary"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                "note.create".to_owned(),
                "session:owner".to_owned(),
                "Created memo \"Roof\"".to_owned()
            ),
            (
                "note.update".to_owned(),
                "session:owner".to_owned(),
                "Edited memo \"Roof plan B\"".to_owned()
            ),
            (
                "note.update".to_owned(),
                "token:admin".to_owned(),
                "Edited memo \"Roof plan C\"".to_owned()
            ),
        ]
    );
    assert_eq!(ours[1]["detail"]["edits"], 2);
    assert_eq!(ours[1]["subject_kind"], "note");
    assert_eq!(ours[1]["detail"]["title"], "Roof plan B");
    assert_eq!(ours[1]["category"], "notes");

    let defs = api.entries("category=definitions&actor=token:admin");
    let idea: Vec<(&str, &str)> = defs
        .iter()
        .filter(|e| e["subject"] == "idea")
        .map(|e| (e["kind"].as_str().unwrap(), e["summary"].as_str().unwrap()))
        .collect();
    assert_eq!(
        idea,
        [
            ("type.create", "Created type idea"),
            ("type.update", "Changed type idea")
        ]
    );
    assert_eq!(defs.last().unwrap()["subject_kind"], "type");

    let auth = api.entries("category=auth");
    let failed = find(&auth, "auth.login_failed");
    assert_eq!(actor(failed), "server:server");
    let login = find(&auth, "auth.login");
    assert_eq!(actor(login), "session:owner");
    assert_eq!(login["detail"]["user_agent"], "activity-test");
    let all = serde_json::to_string(&api.entries("")).unwrap();
    assert!(!all.contains("not the password"));
    assert!(!all.contains(PASSWORD));
}

#[test]
fn the_log_needs_admin_and_pages_with_before() {
    let api = Api::start();
    for i in 0..5 {
        api.memo(&format!("Memo {i}"), "open");
    }
    let reader = api.token("reader", "read");
    let writer = api.token("writer", "write");
    for t in [&reader, &writer] {
        let res = api.req_as(t, "GET", "/api/activity").send().unwrap();
        assert_eq!(res.status().as_u16(), 403);
    }
    let first = api.get("/api/activity?category=notes&limit=2");
    let items = first["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["summary"], "Created memo \"Memo 4\"");
    assert_eq!(items[1]["summary"], "Created memo \"Memo 3\"");
    let next = first["next"].as_str().unwrap();
    assert_eq!(next, items[1]["id"].as_str().unwrap());
    let second = api.get(&format!(
        "/api/activity?category=notes&limit=2&before={next}"
    ));
    assert_eq!(second["items"][0]["summary"], "Created memo \"Memo 2\"");
    let rest = api.get(&format!(
        "/api/activity?category=notes&limit=200&before={}",
        second["next"].as_str().unwrap()
    ));
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert_eq!(rest["next"], Value::Null);
    // Search and kind filters.
    let found = api.get("/api/activity?q=memo%203");
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    let created = api.get("/api/activity?kind=note&limit=200");
    assert_eq!(created["items"].as_array().unwrap().len(), 5);
    let res = api
        .req("GET", "/api/activity?category=nope")
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 400);

    // The token entries: made and revoked, by the admin token.
    expect(api.req("DELETE", "/api/tokens/reader").send().unwrap(), 204);
    let auth = api.entries("category=auth");
    let made = find(&auth, "auth.token_created");
    assert_eq!(actor(made), "token:admin");
    assert_eq!(made["summary"], "Created the read token reader");
    let revoked = find(&auth, "auth.token_revoked");
    assert_eq!(revoked["subject"], "reader");
    assert!(!serde_json::to_string(&auth).unwrap().contains(&reader));
}

#[test]
fn a_chat_turn_records_its_tool_calls_and_tokens_but_no_message_text() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, version) = api.memo("Groceries", "open");
    fake.push(
        AnthropicMessage::new()
            .tool_use(
                "toolu_1",
                "search_notes",
                json!({ "text": "groceries", "limit": 5 }),
            )
            .usage(100, 20),
    );
    fake.push(
        AnthropicMessage::new()
            .tool_use(
                "toolu_2",
                "propose_edit",
                json!({
                    "id": id, "base_version": version,
                    "note": json!({ "fields": { "title": "Groceries and eggs", "status": "open" } }).to_string(),
                }),
            )
            .usage(200, 30),
    );
    fake.push(AnthropicMessage::text("SECRET-ANSWER-TEXT proposed").usage(300, 40));
    let events = api.chat(json!({ "messages": [user("SECRET-QUESTION-TEXT add eggs")] }));
    let proposal = events
        .iter()
        .find(|(n, _)| n == "proposal")
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| panic!("{events:?}"));

    let ai = api.entries("category=ai");
    assert_eq!(
        kinds(&ai),
        [
            "ai.turn.start",
            "ai.tool",
            "ai.proposal",
            "ai.tool",
            "ai.turn.end"
        ],
        "{ai:#?}"
    );
    let turn = ai[0]["detail"]["turn"].as_str().unwrap().to_owned();
    assert!(ai.iter().all(|e| e["detail"]["turn"] == turn.as_str()));
    assert!(ai.iter().all(|e| e["category"] == "ai"));
    assert_eq!(ai[0]["detail"]["origin"], "user");
    assert_eq!(actor(&ai[0]), "token:admin");
    let search = &ai[1];
    assert_eq!(search["detail"]["tool"], "search_notes");
    assert_eq!(search["detail"]["args"]["text"], "groceries");
    assert_eq!(search["summary"], "search_notes \"groceries\" limit=5");
    assert_eq!(search["detail"]["is_error"], false);
    let edit = &ai[3];
    assert_eq!(edit["detail"]["tool"], "propose_edit");
    assert_eq!(edit["detail"]["args"]["id"], id.as_str());
    assert_eq!(edit["detail"]["notes"][0]["title"], "Groceries and eggs");
    assert_eq!(
        ai[2]["summary"],
        "Proposed: update note \"Groceries and eggs\""
    );
    let end = &ai[4];
    assert_eq!(end["detail"]["outcome"], "done");
    assert_eq!(end["detail"]["input_tokens"], 600);
    assert_eq!(end["detail"]["output_tokens"], 90);
    assert_eq!(end["detail"]["model"], MODEL);
    assert_eq!(end["detail"]["account"], "Anthropic");
    assert_eq!(end["detail"]["kind"], "anthropic");
    assert_eq!(end["detail"]["tools"], 2);

    // Applying the proposal: an ai.applied entry, and the note's update by the caller.
    api.send("POST", "/api/ai/apply", proposal);
    let applied = api.entries("kind=ai.applied");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0]["detail"]["ok"], true);
    assert_eq!(
        applied[0]["summary"],
        "Applied: update note \"Groceries and eggs\""
    );
    let notes = api.entries("category=notes&kind=note.update");
    assert_eq!(
        notes.last().unwrap()["summary"],
        "Edited memo \"Groceries and eggs\""
    );

    // A refusal ends its turn as refused.
    fake.push(AnthropicMessage::text("No").refusal(Some("cyber"), Some("Not that.")));
    api.chat(json!({ "messages": [user("SECRET-QUESTION-TEXT something risky")] }));
    let ends = api.entries("kind=ai.turn.end");
    let refused = ends.last().unwrap();
    assert_eq!(refused["detail"]["outcome"], "refused");
    assert_eq!(refused["detail"]["why"], "cyber");

    let all = serde_json::to_string(&api.entries("")).unwrap();
    assert!(!all.contains("SECRET-QUESTION-TEXT"), "{all}");
    assert!(!all.contains("SECRET-ANSWER-TEXT"), "{all}");
    assert!(!all.contains(fake.api_key()));
}

const CLERK: &str = r#"label = "Clerk"

permissions {
  confirm {
    notes = false
  }
}

trigger opened {
  on     = ["create"]
  type   = "memo"
  where  = "status == \"open\""
  prompt = "Tidy this memo."
}
"#;

#[test]
fn a_trigger_run_records_its_turn_the_agents_edit_and_the_run() {
    let fake = FakeAnthropic::start();
    let api = Api::start();
    api.anthropic(&fake);
    expect(
        api.req("PUT", "/api/agents/clerk")
            .header("content-type", "text/plain")
            .body(CLERK)
            .send()
            .unwrap(),
        200,
    );
    let (id, version) = api.memo("Roof", "open");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_e",
        "propose_edit",
        json!({
            "id": id, "base_version": version,
            "note": json!({ "fields": { "title": "Roof (tidied)", "status": "open" } }).to_string(),
        }),
    ));
    fake.push(AnthropicMessage::text("Tidied."));
    api.advance("PT35S");
    assert_eq!(fake.message_requests().len(), 2);

    let defs = api.entries("category=definitions&kind=agent");
    assert_eq!(defs.last().unwrap()["summary"], "Created agent clerk");
    assert_eq!(actor(defs.last().unwrap()), "token:admin");

    let by_agent = api.entries("actor=agent:clerk");
    let start = find(&by_agent, "ai.turn.start");
    assert_eq!(start["detail"]["origin"], "trigger:opened");
    assert_eq!(start["subject_kind"], "thread");
    let thread = start["subject"].as_str().unwrap();
    let edit = by_agent
        .iter()
        .find(|e| e["kind"] == "note.update" && e["subject"] == id.as_str())
        .unwrap_or_else(|| panic!("{by_agent:#?}"));
    assert_eq!(edit["subject"], id.as_str());
    assert_eq!(edit["summary"], "Edited memo \"Roof (tidied)\"");
    assert_eq!(find(&by_agent, "ai.applied")["detail"]["note"], id.as_str());
    let run = find(&by_agent, "agent.run");
    assert_eq!(run["category"], "automation");
    assert_eq!(run["detail"]["outcome"], "succeeded");
    assert_eq!(run["detail"]["trigger"], "opened");
    assert_eq!(run["subject"], thread);
    assert_eq!(find(&by_agent, "ai.turn.end")["detail"]["outcome"], "done");
    // The agent writing its thread note as the turn ran is not logged as a note edit: the turn's
    // own entries say what happened.
    assert!(
        !by_agent
            .iter()
            .any(|e| e["category"] == "notes" && e["subject"] == thread),
        "{by_agent:#?}"
    );
}

#[test]
fn memory_changes_record_their_title_and_never_their_text() {
    let api = Api::start();
    let made = api.send(
        "POST",
        "/api/agents/assistant/memories",
        json!({ "title": "Likes slate", "text": "MEMORY-TEXT-NOT-LOGGED" }),
    );
    let id = made["id"].as_str().unwrap();
    expect(
        api.req("DELETE", &format!("/api/agents/assistant/memories/{id}"))
            .send()
            .unwrap(),
        200,
    );
    let memory = api.entries("kind=ai.memory");
    assert_eq!(kinds(&memory), ["ai.memory", "ai.memory"]);
    assert_eq!(
        memory[0]["summary"],
        "Remembered \"Likes slate\" of assistant"
    );
    assert_eq!(memory[0]["detail"]["op"], "remember");
    assert_eq!(memory[1]["detail"]["op"], "forget");
    assert_eq!(actor(&memory[0]), "token:admin");
    assert_eq!(memory[0]["subject"], id);
    let all = serde_json::to_string(&api.entries("")).unwrap();
    assert!(!all.contains("MEMORY-TEXT-NOT-LOGGED"));
}

#[test]
fn a_task_run_records_its_start_and_outcome_with_the_run_id() {
    let api = Api::start();
    api.send(
        "PUT",
        "/api/scripts/tick",
        json!({ "source": "use vault\nfn run() { vault::log(\"tick\") }\n" }),
    );
    expect(
        api.req("PUT", "/api/tasks/ticker")
            .body("schedule = \"0 * * * *\"\nscript = \"tick\"\n")
            .send()
            .unwrap(),
        200,
    );
    let run = expect(
        api.req("POST", "/api/tasks/ticker/run?wait=true")
            .send()
            .unwrap(),
        200,
    );
    let run_id = run["id"].as_i64().unwrap().to_string();
    let auto = api.entries("category=automation");
    let runs: Vec<&Value> = auto
        .iter()
        .filter(|e| e["subject"] == run_id.as_str())
        .collect();
    assert_eq!(runs.len(), 2, "{auto:#?}");
    assert_eq!(runs[0]["kind"], "task.run.start");
    assert_eq!(runs[1]["kind"], "task.run.end");
    assert_eq!(runs[1]["detail"]["outcome"], "succeeded");
    assert_eq!(runs[1]["detail"]["task"], "ticker");
    assert_eq!(runs[1]["subject_kind"], "task_run");
    assert_eq!(actor(runs[1]), "task:ticker");
    let defs = api.entries("category=definitions");
    assert!(defs.iter().any(|e| e["summary"] == "Created task ticker"));
    assert!(defs.iter().any(|e| e["summary"] == "Created script tick"));
}

/// Pair an Android phone that may send SMS: its id and token.
fn phone(api: &Api, name: &str) -> (String, String) {
    let paired = expect(
        Client::new()
            .post(api.server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": name, "platform": "android" }))
            .send()
            .unwrap(),
        201,
    );
    let id = paired["device_id"].as_str().unwrap().to_owned();
    expect(
        api.req("PATCH", &format!("/api/devices/{id}"))
            .json(&json!({ "settings": { "commands": ["send_sms"], "sms_send": true } }))
            .send()
            .unwrap(),
        200,
    );
    (id, paired["token"].as_str().unwrap().to_owned())
}

#[test]
fn an_outbox_send_records_each_status_with_its_recipient_and_not_its_body() {
    let api = Api::start();
    let (device, token) = phone(&api, "Pixel");
    let item = api.send(
        "POST",
        "/api/notes?type=outbox",
        json!({ "note": {
            "fields": { "title": "SMS to Ann", "kind": "sms", "status": "draft", "phone": "+61400000002" },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
                { "kind": "paragraph", "fields": { "text": "BODY-NOT-LOGGED" } }
            ] }],
        } }),
    );
    let id = item["id"].as_str().unwrap().to_owned();
    // The user marks it ready: the send handler moves it to sending.
    let mut note = api.get(&format!("/api/notes/{id}"));
    note["note"]["fields"]["status"] = json!("ready");
    api.send(
        "PUT",
        &format!("/api/notes/{id}"),
        json!({ "note": note["note"], "base_version": note["version"] }),
    );
    assert!(api.server.settle_handlers().is_empty());
    let pending = expect(
        api.req_as(
            &token,
            "GET",
            &format!("/api/devices/{device}/commands?status=pending"),
        )
        .send()
        .unwrap(),
        200,
    );
    let cid = pending["commands"][0]["id"].as_str().unwrap().to_owned();
    expect(
        api.req_as(
            &token,
            "POST",
            &format!("/api/devices/{device}/commands/{cid}/result"),
        )
        .json(&json!({ "status": "done", "result": { "device_ref": "sms:1" } }))
        .send()
        .unwrap(),
        200,
    );
    let outbox = api.entries("category=outbox");
    let ours: Vec<&Value> = outbox
        .iter()
        .filter(|e| e["subject"] == id.as_str())
        .collect();
    let got: Vec<(&str, String)> = ours
        .iter()
        .map(|e| (e["kind"].as_str().unwrap(), actor(e)))
        .collect();
    assert_eq!(
        got,
        [
            ("outbox.drafted", "token:admin".to_owned()),
            ("outbox.ready", "token:admin".to_owned()),
            ("outbox.sending", "handler:outbox_send".to_owned()),
            ("outbox.sent", "handler:outbox_send".to_owned()),
        ]
    );
    assert_eq!(
        ours[0]["summary"],
        "Drafted sms \"SMS to Ann\" to +61400000002"
    );
    assert_eq!(
        ours[3]["summary"],
        "Sent sms \"SMS to Ann\" to +61400000002"
    );
    assert_eq!(ours[3]["subject_kind"], "outbox");
    assert_eq!(ours[3]["detail"]["to"][0], "+61400000002");
    let all = serde_json::to_string(&api.entries("")).unwrap();
    assert!(!all.contains("BODY-NOT-LOGGED"));
    assert!(!all.contains(&token));

    // The pairing itself.
    let auth = api.entries("category=auth");
    let paired = find(&auth, "auth.device_paired");
    assert_eq!(paired["subject"], device.as_str());
    assert_eq!(actor(paired), "device:Pixel");
}

#[test]
fn a_device_reports_its_sync_rounds() {
    let api = Api::start();
    let (device, token) = phone(&api, "Pixel");
    let (other, other_token) = phone(&api, "Tablet");
    let path = format!("/api/devices/{device}/sync-rounds");
    // A round that did nothing is not recorded.
    expect(
        api.req_as(&token, "POST", &path)
            .json(&json!({ "pushed": 0, "pulled": 0, "held": [] }))
            .send()
            .unwrap(),
        204,
    );
    expect(
        api.req_as(&token, "POST", &path)
            .json(&json!({
                "pushed": 3, "merged": 1, "conflicts": 0, "pulled": 12,
                "held": [{ "note": "01J00000000000000000000000", "asset": null, "reason": "refused" }],
                "something_newer": true,
            }))
            .send()
            .unwrap(),
        204,
    );
    // Another device may not report for it.
    expect(
        api.req_as(&other_token, "POST", &path)
            .json(&json!({ "pushed": 1 }))
            .send()
            .unwrap(),
        403,
    );
    let sync = api.entries("category=sync");
    assert_eq!(sync.len(), 1, "{sync:#?}");
    let round = &sync[0];
    assert_eq!(round["kind"], "sync.round");
    assert_eq!(actor(round), "device:Pixel");
    assert_eq!(
        round["summary"],
        "Pixel synced: pushed 3, pulled 12, 1 held"
    );
    assert_eq!(round["subject_kind"], "device");
    assert_eq!(round["subject"], device.as_str());
    assert_eq!(round["detail"]["held"][0]["reason"], "refused");
    assert_eq!(round["detail"]["merged"], 1);
    let _ = other;
}

/// The `activity` frames of `/api/events` read with `token`, on a thread of their own.
fn activity_frames(api: &Api, token: &str) -> mpsc::Receiver<Value> {
    let res = api
        .client
        .get(api.server.url("/api/events"))
        .bearer_auth(token)
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut name = String::new();
        for line in BufReader::new(res).lines() {
            let Ok(line) = line else { return };
            if let Some(v) = line.strip_prefix("event:") {
                name = v.trim().to_owned();
            } else if let Some(v) = line.strip_prefix("data:")
                && name == "activity"
                && tx.send(serde_json::from_str(v.trim()).unwrap()).is_err()
            {
                return;
            }
        }
    });
    rx
}

#[test]
fn new_entries_are_live_events_for_admin_callers_only() {
    let api = Api::start();
    let writer = api.token("writer", "write");
    let admin = activity_frames(&api, api.server.admin_token());
    let other = activity_frames(&api, &writer);
    // Let both streams open before the write.
    std::thread::sleep(Duration::from_millis(500));
    let (id, _) = api.memo("Live", "open");
    let wait = Duration::from_secs(20);
    let frame = loop {
        let f = admin.recv_timeout(wait).expect("an activity event");
        if f["item"]["kind"] == "note.create" {
            break f;
        }
    };
    let item = &frame["item"];
    assert_eq!(item["subject"], id.as_str());
    assert_eq!(item["summary"], "Created memo \"Live\"");
    assert_eq!(item["actor_kind"], "token");
    // The item is as the list has it.
    let listed = api.get("/api/activity?kind=note.create");
    assert_eq!(&listed["items"][0], item);
    assert!(
        other.recv_timeout(Duration::from_millis(1500)).is_err(),
        "a write token gets no activity events"
    );
}
