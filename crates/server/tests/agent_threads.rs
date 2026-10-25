//! S1 with the fake Anthropic: turns run by an agent on a stored thread (agents G4, G6, G7, G8,
//! G11). The agent's model, prompt, tools and switches; its note types in every note tool; the
//! thread note written as the turn streams and read back by the server; proposals and their
//! outcomes kept in the thread; the thread routes; and the Assistant holding the chat settings.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic, TestServer};

const MEMO: &str = r#"label = "Memo"

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
  }
}
"#;

const LOG: &str = r#"label = "Log"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const MODEL: &str = "claude-opus-5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
            std::fs::write(vault.join("types/log.wcl"), LOG).unwrap();
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let api = Api { server, client };
        api.send(
            "PUT",
            "/api/secrets/ai_anthropic_api_key",
            json!({ "value": fake.api_key() }),
        );
        api.send(
            "PUT",
            "/api/secrets/ai_settings",
            json!({ "value": json!({
                "default_provider": "anthropic",
                "models": { "anthropic": MODEL },
                "endpoints": { "anthropic": fake.base_url() },
            }).to_string() }),
        );
        api
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let v = self.send(
            "POST",
            "/api/tokens",
            json!({ "name": name, "scope": scope }),
        );
        v["secret"].as_str().unwrap().to_owned()
    }

    fn request_as(&self, token: &str, method: &str, path: &str, body: Option<Value>) -> Response {
        let url = self.server.url(path);
        let req = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            "DELETE" => self.client.delete(url),
            m => panic!("{m}"),
        }
        .bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        req.send().unwrap()
    }

    /// `method` on `path` as the admin, which must succeed: its JSON answer.
    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let res = self.request_as(&self.token(), method, path, Some(body));
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        let res = self.request_as(&self.token(), "GET", path, None);
        assert!(
            res.status().is_success(),
            "GET {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap()
    }

    fn agent(&self, name: &str, wcl: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/agents/{name}")))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(wcl.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn thread(&self, agent: &str) -> String {
        let res = self.request_as(
            &self.token(),
            "POST",
            &format!("/api/agents/{agent}/threads"),
            Some(json!({})),
        );
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        v["id"].as_str().unwrap().to_owned()
    }

    fn read_thread(&self, agent: &str, id: &str) -> Value {
        self.get(&format!("/api/agents/{agent}/threads/{id}"))
    }

    fn note(&self, type_name: &str, title: &str) -> (String, String) {
        let v = self.send(
            "POST",
            &format!("/api/notes?type={type_name}"),
            json!({ "note": { "fields": { "title": title } } }),
        );
        (
            v["id"].as_str().unwrap().to_owned(),
            v["version"].as_str().unwrap().to_owned(),
        )
    }

    fn chat_as(&self, token: &str, body: Value) -> Vec<(String, Value)> {
        let res = self.request_as(token, "POST", "/api/ai/chat", Some(body));
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    fn chat(&self, body: Value) -> Vec<(String, Value)> {
        self.chat_as(&self.token(), body)
    }

    /// The author of note `id`'s current version.
    fn author(&self, id: &str) -> Value {
        let history = self.get(&format!("/api/notes/{id}/history"));
        history["versions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["current"] == true)
            .unwrap()["author"]
            .clone()
    }
}

fn read_events(res: Response) -> Vec<(String, Value)> {
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

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn events_named<'a>(events: &'a [(String, Value)], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, d)| d)
        .collect()
}

/// The tool result request `request` sent back for the call before it, as the fake saw it.
fn tool_result(fake: &FakeAnthropic, request: usize) -> Value {
    let r = fake.message_requests()[request].json();
    r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone()
}

fn tool_names(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

const RESEARCHER: &str = r#"label         = "Researcher"
provider      = "anthropic"
model         = "claude-sonnet-5"
effort        = "high"
system_prompt = "You research the vault."

permissions {
  tools = ["search_notes", "get_note"]
}
"#;

#[test]
fn a_thread_turn_uses_the_agents_model_prompt_and_tools_and_is_written_as_it_streams() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("researcher", RESEARCHER);
    let (memo, _) = api.note("memo", "Roof decision");
    let thread = api.thread("researcher");
    let fresh = api.read_thread("researcher", &thread);
    assert_eq!(fresh["title"], "New thread");
    assert_eq!(fresh["messages"], json!([]));

    fake.push(AnthropicMessage::new().tool_use(
        "toolu_s",
        "search_notes",
        json!({ "text": "roof", "limit": 5 }),
    ));
    fake.push(AnthropicMessage::text("Found the roof note."));
    let events = api.chat(json!({
        "agent": "researcher",
        "thread": thread,
        "messages": [user("What did I decide about the roof?")],
    }));
    assert!(events.iter().any(|(n, _)| n == "done"), "{events:?}");

    let req = fake.message_requests()[0].json();
    assert_eq!(req["model"], "claude-sonnet-5");
    assert!(
        req["system"]
            .as_str()
            .unwrap()
            .starts_with("You research the vault."),
        "{req}"
    );
    assert_eq!(req["output_config"]["effort"], "high");
    assert_eq!(tool_names(&fake, 0), ["search_notes", "get_note"]);

    // The thread holds the whole turn, named by its first question, authored by the agent.
    let t = api.read_thread("researcher", &thread);
    assert_eq!(t["title"], "What did I decide about the roof?");
    let messages = t["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4, "{t}");
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[3]["content"][0]["text"], "Found the roof note.");
    assert_eq!(
        t["steps"]["toolu_s"]["summary"]["notes"][0]["id"],
        memo.as_str()
    );
    assert_eq!(
        api.author(&thread),
        json!({ "kind": "agent", "name": "researcher" })
    );
    let note = api.get(&format!("/api/notes/{thread}"));
    let text = note.to_string();
    assert!(text.contains("Found the roof note."), "{text}");
    assert!(text.contains("Used search_notes"), "{text}");
    let list = api.get("/api/agents/researcher/threads");
    assert_eq!(list["threads"][0]["id"], thread.as_str());
    assert_eq!(
        list["threads"][0]["title"],
        "What did I decide about the roof?"
    );
    assert_eq!(list["threads"][0]["pending"], 0);

    // The next turn sends only the new question: the server replays the thread itself.
    fake.push(AnthropicMessage::text("Slate."));
    api.chat(json!({
        "agent": "researcher",
        "thread": thread,
        "messages": [user("Which tiles?")],
    }));
    let replay = fake.message_requests()[2].json();
    let sent = replay["messages"].as_array().unwrap();
    assert_eq!(sent.len(), 5, "{replay}");
    assert_eq!(
        sent[0]["content"][0]["text"],
        "What did I decide about the roof?"
    );
    assert_eq!(sent[4]["content"][0]["text"], "Which tiles?");
    assert_eq!(
        api.read_thread("researcher", &thread)["messages"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    // A tool outside the agent's list is not run, even when the model calls it.
    fake.push(AnthropicMessage::new().tool_use("toolu_l", "list_types", json!({})));
    fake.push(AnthropicMessage::text("No."));
    api.chat(json!({
        "agent": "researcher",
        "thread": thread,
        "messages": [user("List the types.")],
    }));
    let refused = tool_result(&fake, 4);
    assert_eq!(refused["is_error"], true, "{refused}");
}

#[test]
fn the_turns_form_takes_the_agent_and_the_thread() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread("assistant");
    fake.push(AnthropicMessage::text("Hello."));
    let started = api.send(
        "POST",
        "/api/ai/chat/turns",
        json!({ "agent": "assistant", "thread": thread, "messages": [user("Hi")] }),
    );
    let turn = started["turn"].as_str().unwrap();
    let res = api.request_as(
        &api.token(),
        "GET",
        &format!("/api/ai/chat/turns/{turn}/events"),
        None,
    );
    let events = read_events(res);
    assert!(events.iter().any(|(n, _)| n == "done"), "{events:?}");
    let t = api.read_thread("assistant", &thread);
    assert_eq!(t["messages"].as_array().unwrap().len(), 2, "{t}");
    assert_eq!(t["title"], "Hi");
}

const MEMOS_ONLY: &str = r#"label = "Memos"

permissions {
  types = ["memo"]
}
"#;

#[test]
fn every_note_tool_keeps_to_the_agents_types_and_leaves_threads_out() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("memos", MEMOS_ONLY);
    let (memo, _) = api.note("memo", "Roof memo");
    let (log, log_version) = api.note("log", "Roof log");

    let call = |tool: &str, input: Value| -> Value {
        fake.clear_requests();
        fake.push(AnthropicMessage::new().tool_use("toolu_t", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        api.chat(json!({ "agent": "memos", "messages": [user("Go.")] }));
        tool_result(&fake, 1)
    };

    let found = call("search_notes", json!({ "text": "roof", "limit": 10 }));
    let rows: Value = serde_json::from_str(found["content"].as_str().unwrap()).unwrap();
    let ids: Vec<&str> = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [memo.as_str()], "{rows}");
    assert_eq!(rows["left_out"], 1);

    let q = call("query_notes", json!({ "query": "from log" }));
    let rows: Value = serde_json::from_str(q["content"].as_str().unwrap()).unwrap();
    assert_eq!(rows["rows"], json!([]), "{rows}");
    assert_eq!(rows["left_out"], 1);

    let got = call("get_note", json!({ "id": log }));
    assert_eq!(got["is_error"], true);
    assert!(
        got["content"]
            .as_str()
            .unwrap()
            .contains("This agent may not read log notes."),
        "{got}"
    );
    let edit = call(
        "propose_edit",
        json!({ "id": log, "base_version": log_version, "note": json!({ "fields": { "title": "x" } }).to_string() }),
    );
    assert_eq!(edit["is_error"], true);
    assert!(
        edit["content"]
            .as_str()
            .unwrap()
            .contains("may not change log notes")
    );
    let create = call(
        "propose_create",
        json!({ "type": "log", "note": json!({ "fields": { "title": "x" } }).to_string() }),
    );
    assert_eq!(create["is_error"], true);
    let delete = call("propose_delete", json!({ "id": log }));
    assert_eq!(delete["is_error"], true);
    let mine = call("get_note", json!({ "id": memo }));
    assert_eq!(mine["is_error"], false);

    // A thread is left out of the Assistant's searches, as it names no types...
    let thread = api.thread("assistant");
    api.send(
        "PATCH",
        &format!("/api/agents/assistant/threads/{thread}"),
        json!({ "title": "Roof chat" }),
    );
    fake.clear_requests();
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "search_notes",
        json!({ "text": "roof chat", "limit": 10 }),
    ));
    fake.push(AnthropicMessage::text("Done."));
    api.chat(json!({ "messages": [user("Go.")] }));
    let found = tool_result(&fake, 1);
    assert!(
        !found["content"].as_str().unwrap().contains(&thread),
        "{found}"
    );
    // ...and read by an agent whose types name agent_thread.
    api.agent(
        "historian",
        "label = \"Historian\"\n\npermissions {\n  types = [\"agent_thread\"]\n}\n",
    );
    fake.clear_requests();
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "search_notes",
        json!({ "text": "roof chat", "limit": 10 }),
    ));
    fake.push(AnthropicMessage::text("Done."));
    api.chat(json!({ "agent": "historian", "messages": [user("Go.")] }));
    let found = tool_result(&fake, 1);
    assert!(
        found["content"].as_str().unwrap().contains(&thread),
        "{found}"
    );
}

#[test]
fn an_agent_applies_its_own_confirm_switches_within_the_callers_scope() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "scribe",
        "label = \"Scribe\"\n\npermissions {\n  confirm {\n    notes = false\n  }\n}\n",
    );
    let (memo, version) = api.note("memo", "Groceries");
    let edit = |token: &str, agent: Option<&str>, version: &str, title: &str| {
        fake.push(AnthropicMessage::new().tool_use(
            "toolu_e",
            "propose_edit",
            json!({
                "id": memo, "base_version": version,
                "note": json!({ "fields": { "title": title } }).to_string(),
            }),
        ));
        fake.push(AnthropicMessage::text("Done."));
        let mut body = json!({ "messages": [user("Edit it.")] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        api.chat_as(token, body)
    };

    // The Assistant asks first: its own switch is on.
    let events = edit(&api.token(), None, &version, "Groceries and more");
    assert_eq!(events_named(&events, "proposal").len(), 1);
    assert!(events_named(&events, "applied").is_empty());

    // The scribe writes at once, authored by itself.
    let events = edit(&api.token(), Some("scribe"), &version, "Groceries and more");
    let applied = events_named(&events, "applied");
    assert_eq!(applied.len(), 1, "{events:?}");
    assert_eq!(applied[0]["agent"], "scribe");
    assert_eq!(
        api.author(&memo),
        json!({ "kind": "agent", "name": "scribe" })
    );

    // A caller with read scope only gets a proposal, whatever the agent's switch says.
    let reader = api.token_of("reader", "read");
    let now = api.get(&format!("/api/notes/{memo}"))["version"]
        .as_str()
        .unwrap()
        .to_owned();
    let events = edit(&reader, Some("scribe"), &now, "Groceries for Monday");
    assert_eq!(events_named(&events, "proposal").len(), 1, "{events:?}");
    assert!(events_named(&events, "applied").is_empty());
}

#[test]
fn proposals_and_their_outcomes_are_kept_in_the_thread_and_told_to_the_model_once() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let (memo, version) = api.note("memo", "Groceries");
    let thread = api.thread("assistant");
    fake.push(
        AnthropicMessage::new()
            .tool_use(
                "toolu_a",
                "propose_edit",
                json!({
                    "id": memo, "base_version": version,
                    "note": json!({ "fields": { "title": "Groceries for Sunday" } }).to_string(),
                }),
            )
            .tool_use(
                "toolu_b",
                "propose_create",
                json!({ "type": "memo", "note": json!({ "fields": { "title": "Hardware" } }).to_string() }),
            ),
    );
    fake.push(AnthropicMessage::text("Two changes for you."));
    let events = api.chat(json!({
        "agent": "assistant", "thread": thread, "messages": [user("Tidy up.")],
    }));
    let proposals = events_named(&events, "proposal");
    assert_eq!(proposals.len(), 2, "{events:?}");
    assert_eq!(proposals[0]["thread"], thread.as_str());
    assert_eq!(proposals[0]["agent"], "assistant");

    // Reopened, the thread holds both, pending, and the list shows the badge count.
    let t = api.read_thread("assistant", &thread);
    let stored = t["proposals"].as_array().unwrap();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|p| p["status"] == "pending"), "{t}");
    assert_eq!(t["pending"], 2);
    assert_eq!(
        api.get("/api/agents/assistant/threads")["threads"][0]["pending"],
        2
    );

    // Apply the edit, as the Assistant; discard the create.
    let done = api.send("POST", "/api/ai/apply", proposals[0].clone());
    assert_eq!(done["op"], "update");
    assert_eq!(
        api.author(&memo),
        json!({ "kind": "agent", "name": "assistant" })
    );
    api.send(
        "POST",
        "/api/ai/discard",
        json!({ "id": proposals[1]["id"], "thread": thread }),
    );
    let t = api.read_thread("assistant", &thread);
    assert_eq!(t["proposals"][0]["status"], "applied");
    assert_eq!(t["proposals"][0]["result"]["version"], done["version"]);
    assert_eq!(t["proposals"][1]["status"], "discarded");
    assert_eq!(t["pending"], 0);

    // The next turn starts with the outcomes, once.
    fake.clear_requests();
    fake.push(AnthropicMessage::text("Noted."));
    api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("Thanks.")] }));
    let req = fake.message_requests()[0].json();
    let last = req["messages"].as_array().unwrap().last().unwrap().clone();
    let lines = last["content"][0]["text"].as_str().unwrap();
    assert!(
        lines.contains(&format!(
            "[Applied by the user: edit of note {memo} now at version"
        )),
        "{lines}"
    );
    assert!(
        lines.contains("[Discarded by the user: new memo note \"Hardware\"]"),
        "{lines}"
    );
    assert_eq!(last["content"][1]["text"], "Thanks.");
    fake.clear_requests();
    fake.push(AnthropicMessage::text("Sure."));
    api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("Bye.")] }));
    let req = fake.message_requests()[0].json();
    let last = req["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["content"].as_array().unwrap().len(), 1, "{last}");

    // A failed apply is kept as stale: the note moved on.
    let stale = api.request_as(
        &api.token(),
        "POST",
        "/api/ai/apply",
        Some(proposals[0].clone()),
    );
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let t = api.read_thread("assistant", &thread);
    assert_eq!(t["proposals"][0]["status"], "stale");
}

#[test]
fn thread_routes_list_create_get_rename_and_delete() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("researcher", RESEARCHER);
    let reader = api.token_of("reader", "read");

    // A read token starts a thread and reads it; renaming and deleting need write.
    let res = api.request_as(
        &reader,
        "POST",
        "/api/agents/researcher/threads",
        Some(json!({ "title": "Roofing" })),
    );
    assert_eq!(res.status(), StatusCode::CREATED);
    let t: Value = res.json().unwrap();
    let id = t["id"].as_str().unwrap().to_owned();
    assert_eq!(t["title"], "Roofing");
    assert_eq!(t["origin"], "user");
    assert_eq!(
        api.author(&id),
        json!({ "kind": "agent", "name": "researcher" })
    );
    let res = api.request_as(
        &reader,
        "GET",
        &format!("/api/agents/researcher/threads/{id}"),
        None,
    );
    assert_eq!(res.status(), StatusCode::OK);
    let res = api.request_as(
        &reader,
        "PATCH",
        &format!("/api/agents/researcher/threads/{id}"),
        Some(json!({ "title": "Nope" })),
    );
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let older = api.thread("researcher");
    std::thread::sleep(Duration::from_millis(1100));
    let renamed = api.send(
        "PATCH",
        &format!("/api/agents/researcher/threads/{older}"),
        json!({ "title": "Slate" }),
    );
    assert_eq!(renamed["title"], "Slate");
    let list = api.get("/api/agents/researcher/threads");
    let ids: Vec<&str> = list["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [older.as_str(), id.as_str()], "newest activity first");
    // Another agent's thread is not this agent's.
    let res = api.request_as(
        &api.token(),
        "GET",
        &format!("/api/agents/assistant/threads/{id}"),
        None,
    );
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    // A thread of another agent cannot carry this agent's turn.
    let res = api.request_as(
        &api.token(),
        "POST",
        "/api/ai/chat",
        Some(json!({ "agent": "assistant", "thread": id, "messages": [user("Hi")] })),
    );
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    api.send(
        "DELETE",
        &format!("/api/agents/researcher/threads/{id}"),
        json!({}),
    );
    let res = api.request_as(&api.token(), "GET", &format!("/api/notes/{id}"), None);
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let trash = api.get("/api/trash");
    assert!(trash.to_string().contains(&id), "{trash}");
    assert_eq!(
        api.get("/api/agents/researcher/threads")["threads"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn archived_threads_are_listed_only_when_asked_and_sending_in_one_unarchives_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("researcher", RESEARCHER);
    let kept = api.thread("researcher");
    let filed = api.thread("researcher");
    let version = api.read_thread("researcher", &filed)["version"]
        .as_str()
        .unwrap()
        .to_owned();
    // Archiving is the ordinary note route, authored by the caller, not the agent.
    api.send(
        "POST",
        &format!("/api/notes/{filed}/archive?base_version={version}"),
        json!({}),
    );
    assert_eq!(
        api.author(&filed),
        json!({ "kind": "token", "name": "admin" })
    );

    let ids = |path: &str| -> Vec<(String, bool)> {
        api.get(path)["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["id"].as_str().unwrap().to_owned(),
                    t["archived"].as_bool().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(
        ids("/api/agents/researcher/threads"),
        [(kept.clone(), false)]
    );
    assert_eq!(
        ids("/api/agents/researcher/threads?archived=false"),
        [(kept.clone(), false)]
    );
    let all = ids("/api/agents/researcher/threads?archived=true");
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(all.contains(&(filed.clone(), true)), "{all:?}");
    assert!(all.contains(&(kept.clone(), false)), "{all:?}");
    let res = api.request_as(
        &api.token(),
        "GET",
        "/api/agents/researcher/threads?archived=maybe",
        None,
    );
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // An archived thread still opens, and sending in it takes it out of the archive.
    let t = api.read_thread("researcher", &filed);
    assert_eq!(t["archived"], true);
    std::thread::sleep(Duration::from_millis(1100));
    fake.push(AnthropicMessage::text("Back again."));
    let events = api.chat(json!({
        "agent": "researcher",
        "thread": filed,
        "messages": [user("Pick this up again")],
    }));
    assert!(events.iter().any(|(n, _)| n == "done"), "{events:?}");
    let t = api.read_thread("researcher", &filed);
    assert_eq!(t["archived"], false);
    assert_eq!(t["messages"].as_array().unwrap().len(), 2, "{t}");
    let listed = ids("/api/agents/researcher/threads");
    assert_eq!(listed[0], (filed.clone(), false), "{listed:?}");
}

#[test]
fn a_disabled_agent_takes_no_turn() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("sleepy", "label = \"Sleepy\"\nenabled = false\n");
    let res = api.request_as(
        &api.token(),
        "POST",
        "/api/ai/chat",
        Some(json!({ "agent": "sleepy", "messages": [user("Hi")] })),
    );
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v: Value = res.json().unwrap();
    assert_eq!(v["error"], "agent_disabled");
    let res = api.request_as(
        &api.token(),
        "POST",
        "/api/ai/chat",
        Some(json!({ "agent": "nobody", "messages": [user("Hi")] })),
    );
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test]
fn the_chat_settings_are_the_assistants_and_providers_answers_them() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "assistant",
        "label = \"Assistant\"\nsystem_prompt = \"Be brief.\"\n\npermissions {\n  confirm {\n    delete = false\n  }\n\n  web {\n    read = false\n  }\n}\n",
    );
    let reader = api.token_of("reader", "read");
    let res = api.request_as(&reader, "GET", "/api/ai/providers", None);
    let p: Value = res.json().unwrap();
    assert_eq!(p["confirm"]["delete"], false, "{p}");
    assert_eq!(p["confirm"]["notes"], true);
    assert_eq!(p["web"]["read"], false);
    assert_eq!(p["web"]["search"], true);
    let status = api.get("/api/integrations/ai");
    assert_eq!(status["system_prompt"], "Be brief.");
    assert_eq!(status["system_prompt_custom"], true);
    assert_eq!(status["confirm"]["delete"], false);

    fake.push(AnthropicMessage::text("Hi."));
    api.chat(json!({ "messages": [user("Hello")] }));
    let req = fake.message_requests()[0].json();
    assert!(
        req["system"].as_str().unwrap().starts_with("Be brief."),
        "{req}"
    );
    assert!(!tool_names(&fake, 0).contains(&"fetch_url".to_owned()));

    // Settings > AI's old keys write the Assistant too.
    api.send(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "confirm": { "delete": true }, "web": { "read": true } }),
    );
    let a = api.get("/api/agents/assistant");
    assert_eq!(a["permissions"]["confirm"]["delete"], true);
    assert_eq!(a["permissions"]["web"]["read"], true);
    assert_eq!(a["system_prompt"], "Be brief.");
}

#[test]
fn an_app_applies_an_agents_type_proposal_from_its_thread_as_the_thread_holds_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // An app's token has `write`, never `admin`.
    let app = api.token_of("phone", "write");
    let thread = api.thread("assistant");
    let changed = MEMO.replace("tab \"Main\"", "tab \"Details\"");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "propose_type_change",
        json!({ "name": "memo", "wcl": changed }),
    ));
    fake.push(AnthropicMessage::text("A change for you."));
    let events = api.chat_as(
        &app,
        json!({ "agent": "assistant", "thread": thread, "messages": [user("Rename the tab.")] }),
    );
    let proposal = events_named(&events, "type_proposal")[0].clone();
    assert_eq!(proposal["thread"], thread.as_str());
    let type_file = || {
        api.get("/api/types/memo")["wcl"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // The body sent is not what is applied: the thread's copy is.
    let mut tampered = proposal.clone();
    tampered["after"] = json!(MEMO.replace("tab \"Main\"", "tab \"Mine\""));
    let res = api.request_as(&app, "POST", "/api/ai/apply", Some(tampered));
    assert_eq!(res.status(), StatusCode::OK);
    let done: Value = res.json().unwrap();
    assert_eq!(done, json!({ "op": "type_change", "type_name": "memo" }));
    assert!(type_file().contains("tab \"Details\""), "{}", type_file());
    let t = api.read_thread("assistant", &thread);
    assert_eq!(t["proposals"][0]["status"], "applied", "{t}");

    // Applied, it no longer waits: a second apply needs admin.
    let res = api.request_as(&app, "POST", "/api/ai/apply", Some(proposal.clone()));
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let err: Value = res.json().unwrap();
    assert_eq!(err["error"], "needs_admin");

    // A change no thread holds needs admin as before.
    let mut loose = proposal;
    loose["thread"] = Value::Null;
    loose["id"] = json!("toolu_other");
    loose["before"] = json!(type_file());
    loose["after"] = json!(MEMO);
    let res = api.request_as(&app, "POST", "/api/ai/apply", Some(loose));
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(type_file().contains("tab \"Details\""));
}

/// The text of every message `request` sent, in order, as `role: text`.
fn sent_texts(request: &Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let text: Vec<&str> = match &m["content"] {
                Value::String(s) => vec![s.as_str()],
                Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect(),
                _ => Vec::new(),
            };
            format!(
                "{}: {}",
                m["role"].as_str().unwrap_or_default(),
                text.join(" | ")
            )
        })
        .collect()
}

#[test]
fn each_call_reports_the_context_and_compact_summarizes_the_thread_for_the_model() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread("assistant");
    fake.push(AnthropicMessage::text("Paris is the capital.").usage(1200, 30));
    let events = api.chat(
        json!({ "agent": "assistant", "thread": thread, "messages": [user("Capital of France?")] }),
    );
    let context = events_named(&events, "context");
    assert_eq!(
        context,
        [&json!({ "used": 1230, "window": 200_000 })],
        "{events:?}"
    );
    let t = api.read_thread("assistant", &thread);
    assert_eq!(t["context"], json!({ "used": 1230, "window": 200_000 }));
    assert_eq!(t["compaction"], Value::Null);

    // /compact: one call for the summary, kept in the thread; the turn ends there.
    fake.clear_requests();
    fake.push(
        AnthropicMessage::text("The user asked for France's capital: Paris.").usage(1300, 12),
    );
    let events = api
        .chat(json!({ "agent": "assistant", "thread": thread, "messages": [], "compact": true }));
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["compacting", "compacted", "context", "done"],
        "{events:?}"
    );
    let compacted = events_named(&events, "compacted")[0];
    assert_eq!(compacted["reason"], "command");
    assert_eq!(compacted["upto"], 2);
    assert_eq!(
        compacted["summary"],
        "The user asked for France's capital: Paris."
    );
    assert_eq!(compacted["before"], 1300);
    let asked = fake.message_requests()[0].json();
    assert_eq!(
        asked["tools"].as_array().map_or(0, Vec::len),
        0,
        "no tools for a summary"
    );
    let texts = sent_texts(&asked);
    assert!(texts[0].contains("Capital of France?"), "{texts:?}");
    assert!(
        texts.last().unwrap().contains("Summarize the conversation"),
        "{texts:?}"
    );
    let t = api.read_thread("assistant", &thread);
    assert_eq!(t["compaction"]["upto"], 2);
    // The thread keeps every message for the user.
    assert_eq!(t["messages"].as_array().unwrap().len(), 2);
    assert_eq!(t["context"], json!({ "used": 12, "window": 200_000 }));

    // The next turn sends the summary in place of the older messages.
    fake.clear_requests();
    fake.push(AnthropicMessage::text("Berlin."));
    api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("And Germany?")] }));
    let texts = sent_texts(&fake.message_requests()[0].json());
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].starts_with("user: [The conversation before this point was compacted."),
        "{texts:?}"
    );
    assert!(texts[0].contains("France's capital: Paris."), "{texts:?}");
    assert!(texts[0].ends_with("And Germany?"), "{texts:?}");
    assert!(!texts[0].contains("Capital of France?"), "{texts:?}");
    assert_eq!(
        api.read_thread("assistant", &thread)["messages"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn a_thread_that_filled_its_window_is_compacted_before_the_next_turn() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.send(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "context": { "windows": { "anthropic": 10_000 }, "compact_at": 60 } }),
    );
    let thread = api.thread("assistant");
    fake.push(AnthropicMessage::text("A long answer.").usage(5_000, 500));
    api.chat(
        json!({ "agent": "assistant", "thread": thread, "messages": [user("Tell me a lot.")] }),
    );
    // 5,500 of 10,000 is under 60%: nothing yet.
    fake.push(AnthropicMessage::text("Longer still.").usage(6_000, 200));
    let events =
        api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("More.")] }));
    assert!(events_named(&events, "compacting").is_empty(), "{events:?}");

    // 6,200 is over: the next turn compacts first, then answers from the summary.
    fake.clear_requests();
    fake.push(AnthropicMessage::text("Summary: the user wanted a lot, twice.").usage(6_300, 9));
    fake.push(AnthropicMessage::text("Here is the rest.").usage(400, 20));
    let events =
        api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("Finish.")] }));
    assert_eq!(
        events_named(&events, "compacting"),
        [&json!({ "reason": "auto" })]
    );
    assert_eq!(events_named(&events, "compacted")[0]["upto"], 4);
    let text: String = events_named(&events, "text")
        .iter()
        .filter_map(|d| d["delta"].as_str())
        .collect();
    assert_eq!(text, "Here is the rest.");
    let requests = fake.message_requests();
    assert_eq!(requests.len(), 2);
    let answer = sent_texts(&requests[1].json());
    assert_eq!(answer.len(), 1, "{answer:?}");
    assert!(
        answer[0].contains("the user wanted a lot, twice") && answer[0].ends_with("Finish."),
        "{answer:?}"
    );

    // Turned off, a full thread is left as it is.
    api.send(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "context": { "auto_compact": false } }),
    );
    fake.push(AnthropicMessage::text("Big.").usage(9_000, 100));
    api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("Again.")] }));
    fake.push(AnthropicMessage::text("Fine."));
    let events = api
        .chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("Once more.")] }));
    assert!(events_named(&events, "compacting").is_empty(), "{events:?}");
    let settings = api.get("/api/integrations/ai");
    assert_eq!(
        settings["context"],
        json!({
            "windows": {
                "anthropic": { "window": 10_000, "set": true },
                "openai": { "window": 400_000, "set": false },
                "chatgpt": { "window": 272_000, "set": false },
            },
            "auto_compact": false,
            "compact_at": 60,
        })
    );
}

#[test]
fn compact_needs_a_thread_and_settings_refuse_a_window_out_of_range() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let res = api.request_as(
        &api.token(),
        "POST",
        "/api/ai/chat",
        Some(json!({ "messages": [], "compact": true })),
    );
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    for body in [
        json!({ "context": { "windows": { "anthropic": 10 } } }),
        json!({ "context": { "compact_at": 99 } }),
    ] {
        let res = api.request_as(
            &api.token(),
            "PUT",
            "/api/integrations/ai/settings",
            Some(body),
        );
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn a_thread_cut_back_to_a_question_drops_it_and_what_followed_and_a_fork_copies_what_came_before() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let (memo, version) = api.note("memo", "Groceries");
    let thread = api.thread("assistant");
    fake.push(AnthropicMessage::text("First answer."));
    api.chat(
        json!({ "agent": "assistant", "thread": thread, "messages": [user("First question")] }),
    );
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_e",
        "propose_edit",
        json!({
            "id": memo, "base_version": version,
            "note": json!({ "fields": { "title": "Groceries for Sunday" } }).to_string(),
        }),
    ));
    fake.push(AnthropicMessage::text("Proposed."));
    api.chat(
        json!({ "agent": "assistant", "thread": thread, "messages": [user("Second question")] }),
    );
    let t = api.read_thread("assistant", &thread);
    // Q1, A1, Q2, the call, its result, the answer.
    assert_eq!(t["messages"].as_array().unwrap().len(), 6);
    assert_eq!(t["pending"], 1);

    // Message 1 is an answer, and 4 a tool result: neither is a question.
    for at in [1, 4, 9] {
        let res = api.request_as(
            &api.token(),
            "POST",
            &format!("/api/agents/assistant/threads/{thread}/truncate"),
            Some(json!({ "at": at })),
        );
        assert_eq!(res.status(), StatusCode::CONFLICT, "at {at}");
    }

    // Forked at the second question: a new thread with the first exchange; the old one keeps all.
    let forked = api.send(
        "POST",
        &format!("/api/agents/assistant/threads/{thread}/fork"),
        json!({ "at": 2 }),
    );
    let fork = forked["id"].as_str().unwrap().to_owned();
    assert_ne!(fork, thread);
    assert!(
        forked["title"].as_str().unwrap().ends_with("(fork)"),
        "{forked}"
    );
    assert_eq!(forked["messages"].as_array().unwrap().len(), 2);
    assert_eq!(forked["proposals"], json!([]));
    assert_eq!(
        api.read_thread("assistant", &thread)["messages"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    // Cut back to the second question: it and its proposal are gone.
    let cut = api.send(
        "POST",
        &format!("/api/agents/assistant/threads/{thread}/truncate"),
        json!({ "at": 2 }),
    );
    assert_eq!(cut["messages"].as_array().unwrap().len(), 2);
    assert_eq!(cut["proposals"], json!([]));
    assert_eq!(cut["pending"], 0);

    // The next turn sends only what is left.
    fake.clear_requests();
    fake.push(AnthropicMessage::text("Again."));
    api.chat(json!({ "agent": "assistant", "thread": thread, "messages": [user("Second question, put better")] }));
    let sent = fake.message_requests()[0].json();
    let texts = sent_texts(&sent);
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert!(
        texts[2].ends_with("Second question, put better"),
        "{texts:?}"
    );
}

#[test]
fn the_chat_boxs_provider_and_model_win_over_the_agents_own() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("researcher", RESEARCHER);
    let models = |fake: &FakeAnthropic| -> Vec<String> {
        fake.message_requests()
            .iter()
            .map(|r| r.json()["model"].as_str().unwrap().to_owned())
            .collect()
    };
    let ask = |extra: Value| {
        fake.push(AnthropicMessage::text("Done."));
        let mut body = json!({ "agent": "researcher", "messages": [user("Hello")] });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let events = api.chat(body);
        assert!(events.iter().any(|(n, _)| n == "done"), "{events:?}");
    };
    // No choice, or an empty one: the agent's own model.
    ask(json!({}));
    ask(json!({ "provider": "", "model": "" }));
    // A model alone runs on the agent's provider.
    ask(json!({ "model": "claude-haiku-5" }));
    // Both, as the chat box sends them.
    ask(json!({ "provider": "anthropic", "model": "claude-fable-5" }));
    // A provider alone takes that provider's default from Settings > AI, not the agent's model.
    ask(json!({ "provider": "anthropic" }));
    assert_eq!(
        models(&fake),
        [
            "claude-sonnet-5",
            "claude-sonnet-5",
            "claude-haiku-5",
            "claude-fable-5",
            MODEL
        ]
    );
}
