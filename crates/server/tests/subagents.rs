//! S1 with the fake Anthropic provider: sub agents (subagents U1 to U8).
//!
//! `start_subagent` is offered only with `permissions.subagents.allow`, on a thread, and never to
//! a sub agent; calls in one round run together up to `max`, and a named agent must be listed. A
//! throwaway sub agent's permissions only narrow, a named one gets what both agents have. Its
//! proposals surface in the parent chat and apply there, authored `agent:<parent>/sub:<n>`, and
//! changes it makes itself are authored the same way. Stopping the parent stops its sub agents;
//! stopping a sub agent answers the parent's call. Sessions are threads listed only under their
//! parent, archived, truncated and deleted with it, and every run is in the activity log.

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

const ITEM: &str = r#"label = "Item"

field done {
  kind = "bool"
}

template default {
  tab "Main" {
    slot "title"
    slot "done"
  }
}
"#;

/// An agent that may start two sub agents at once, and delegate to the writer.
const LEAD: &str = r#"label = "Lead"

permissions {
  tools = ["get_note", "search_notes", "propose_create"]

  subagents {
    allow  = true
    max    = 2
    agents = ["writer"]
  }
}
"#;

/// The lead with note changes made without asking.
const FREE_LEAD: &str = r#"label = "Free lead"

permissions {
  tools = ["get_note", "search_notes", "propose_create"]

  confirm {
    notes = false
  }

  subagents {
    allow  = true
    agents = ["writer"]
  }
}
"#;

const WRITER: &str = r#"label = "Writer"
system_prompt = "You write notes."

permissions {
  tools = ["get_note", "query_notes", "propose_create"]

  confirm {
    notes = false
  }
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/item.wcl"), ITEM).unwrap();
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.send_ok(
                "PUT",
                &format!("/api/secrets/{name}"),
                json!({ "value": value }),
            );
        }
        api.agent("lead", LEAD);
        api.agent("free_lead", FREE_LEAD);
        api.agent("writer", WRITER);
        api
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let v = self.send_ok(
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
            "DELETE" => self.client.delete(url),
            "PUT" => self.client.put(url),
            m => panic!("{m}"),
        }
        .bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        req.send().unwrap()
    }

    fn send(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let res = self.request_as(&self.token(), method, path, body);
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn send_ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, v) = self.send(method, path, Some(body));
        assert!(status.is_success(), "{method} {path}: {status} {v}");
        v
    }

    fn get(&self, path: &str) -> Value {
        let (status, v) = self.send("GET", path, None);
        assert!(status.is_success(), "{path}: {status} {v}");
        v
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
        let v = self.send_ok("POST", &format!("/api/agents/{agent}/threads"), json!({}));
        v["id"].as_str().unwrap().to_owned()
    }

    fn read_thread(&self, agent: &str, id: &str) -> (StatusCode, Value) {
        self.send("GET", &format!("/api/agents/{agent}/threads/{id}"), None)
    }

    /// One turn of `agent` on `thread`, read to its end.
    fn chat(&self, agent: &str, thread: Option<&str>, text: &str) -> Vec<(String, Value)> {
        let mut body = json!({ "agent": agent, "messages": [user(text)] });
        if let Some(t) = thread {
            body["thread"] = json!(t);
        }
        let res = self.request_as(&self.token(), "POST", "/api/ai/chat", Some(body));
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res, |_| false)
    }

    /// Start a turn of `agent` on `thread` in the turns form: its id.
    fn start_turn(&self, agent: &str, thread: &str, text: &str) -> String {
        let v = self.send_ok(
            "POST",
            "/api/ai/chat/turns",
            json!({ "agent": agent, "thread": thread, "messages": [user(text)] }),
        );
        v["turn"].as_str().unwrap().to_owned()
    }

    /// Turn `turn`'s events until `stop` says so or it ends.
    fn turn_events(
        &self,
        turn: &str,
        stop: impl FnMut(&(String, Value)) -> bool,
    ) -> Vec<(String, Value)> {
        let res = self.request_as(
            &self.token(),
            "GET",
            &format!("/api/ai/chat/turns/{turn}/events"),
            None,
        );
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res, stop)
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

    /// Wait until `f` holds, for at most 20 seconds.
    fn wait(&self, what: &str, mut f: impl FnMut() -> bool) {
        let began = Instant::now();
        while !f() {
            assert!(began.elapsed() < Duration::from_secs(20), "{what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn read_events(
    res: Response,
    mut stop: impl FnMut(&(String, Value)) -> bool,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    for line in BufReader::new(res).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                let e = (
                    std::mem::take(&mut name),
                    serde_json::from_str(&data).unwrap(),
                );
                data.clear();
                let done = stop(&e);
                out.push(e);
                if done {
                    break;
                }
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

fn find<'a>(events: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    events.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// The tool names a request offered.
fn offered(request: &Value) -> Vec<String> {
    let mut names: Vec<String> = request["tools"]
        .as_array()
        .map(|t| {
            t.iter()
                .map(|t| t["name"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn system_of(request: &Value) -> String {
    match &request["system"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The tool results of a request's last message, in order, with their error flags.
fn results_of(request: &Value) -> Vec<(bool, Value)> {
    request["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "tool_result")
        .map(|c| {
            let text = match &c["content"] {
                Value::String(s) => s.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<String>(),
                _ => String::new(),
            };
            (
                c["is_error"] == true,
                serde_json::from_str(&text).unwrap_or(Value::String(text)),
            )
        })
        .collect()
}

fn start(id: &str, task: &str) -> (String, &'static str, Value) {
    (
        id.to_owned(),
        "start_subagent",
        json!({ "task": task, "instructions": null, "agent": null, "permissions": null, "model": null }),
    )
}

fn calls(list: &[(String, &str, Value)]) -> AnthropicMessage {
    let mut m = AnthropicMessage::new();
    for (id, name, input) in list {
        m = m.tool_use(id, name, input.clone());
    }
    m
}

fn item(title: &str) -> Value {
    json!({ "type": "item", "note": json!({ "fields": { "title": title } }).to_string() })
}

#[test]
fn start_subagent_is_offered_with_the_permission_on_a_thread_and_never_to_a_sub_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);

    // Without the permission, or without a thread to keep the session, it is not offered.
    fake.push(AnthropicMessage::text("Hi."));
    let t = api.thread("writer");
    api.chat("writer", Some(&t), "Hi");
    let r = fake.message_requests().last().unwrap().json();
    assert!(!offered(&r).contains(&"start_subagent".to_owned()));
    fake.push(AnthropicMessage::text("Hi."));
    api.chat("lead", None, "Hi");
    let r = fake.message_requests().last().unwrap().json();
    assert!(!offered(&r).contains(&"start_subagent".to_owned()));

    // With it, on a thread, it is; the sub agent it starts is not offered it.
    fake.clear_requests();
    let thread = api.thread("lead");
    fake.push(calls(&[start("toolu_0", "Look around")]));
    fake.push(AnthropicMessage::text("Looked: nothing new."));
    fake.push(AnthropicMessage::text("All done."));
    let events = api.chat("lead", Some(&thread), "Check things");
    let requests = fake.message_requests();
    assert_eq!(requests.len(), 3);
    let lead = requests[0].json();
    assert!(offered(&lead).contains(&"start_subagent".to_owned()));
    let tool = lead["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "start_subagent")
        .unwrap();
    assert!(tool["description"].as_str().unwrap().contains("writer"));
    let sub = requests[1].json();
    assert_eq!(
        offered(&sub),
        ["get_note", "propose_create", "search_notes"],
        "the parent's tools, and no start_subagent"
    );
    let system = system_of(&sub);
    assert!(system.contains("## Sub agent"), "{system}");
    assert!(system.contains("Your task: Look around"), "{system}");
    assert_eq!(sub["messages"][0]["content"][0]["text"], "Look around");

    // The parent gets the sub agent's answer, with a status line.
    let (err, out) = &results_of(&requests[2].json())[0];
    assert!(!err, "{out}");
    assert_eq!(out["answer"], "Looked: nothing new.");
    assert_eq!(out["outcome"], "done");
    assert!(out["status"].as_str().unwrap().contains("1 round"), "{out}");
    // The block's live status went running, then done.
    let subs: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "subagent")
        .map(|(_, v)| v)
        .collect();
    assert_eq!(subs.first().unwrap()["status"], "running");
    assert_eq!(subs.last().unwrap()["status"], "done");
    assert_eq!(subs.last().unwrap()["id"], "toolu_0");
    let summary = &events
        .iter()
        .find(|(n, v)| n == "tool_result" && v["id"] == "toolu_0")
        .unwrap()
        .1["summary"]["subagent"];
    assert_eq!(summary["answer"], "Looked: nothing new.");
    assert_eq!(summary["outcome"], "done");

    // An unattended run may start them too (subagents U8).
    let runs = api.get("/api/agents/lead/runs");
    assert!(
        runs["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "start_subagent"),
        "{runs}"
    );
}

#[test]
fn calls_run_together_up_to_max_and_only_listed_agents_are_named() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread("lead");
    let mut critic = start("toolu_3", "Criticise");
    critic.2["agent"] = json!("critic");
    fake.push_when(
        "Split the work",
        calls(&[
            start("toolu_0", "Task A"),
            start("toolu_1", "Task B"),
            start("toolu_2", "Task C"),
            critic,
        ]),
    );
    // Each answer streams for a while, so the two runs overlap only if they run at once.
    fake.push_when(
        "Your task: Task A",
        AnthropicMessage::text("A is done.").delay(Duration::from_millis(40)),
    );
    fake.push_when(
        "Your task: Task B",
        AnthropicMessage::text("B is done.").delay(Duration::from_millis(40)),
    );
    fake.push_when("B is done", AnthropicMessage::text("Both done."));
    let began = Instant::now();
    let events = api.chat("lead", Some(&thread), "Split the work");
    assert!(began.elapsed() < Duration::from_secs(20));
    assert_eq!(
        find(&events, "done").unwrap()["stop"],
        "end_turn",
        "{events:?}"
    );
    let requests = fake.message_requests();
    assert_eq!(requests.len(), 4, "the lead twice and two sub agents");
    let results = results_of(&requests[3].json());
    assert_eq!(results.len(), 4);
    assert!(
        !results[0].0 && results[0].1["answer"] == "A is done.",
        "{results:?}"
    );
    assert!(
        !results[1].0 && results[1].1["answer"] == "B is done.",
        "{results:?}"
    );
    for (err, out) in &results[2..] {
        assert!(err);
        assert!(
            out["error"].as_str().unwrap().contains("at most 2"),
            "{out}"
        );
    }
    // The two sessions overlapped: each began before the other ended.
    let t = api.read_thread("lead", &thread).1;
    let subs = t["subagents"].as_array().unwrap();
    assert_eq!(subs.len(), 2);
    assert!(subs[0]["started_ms"].as_i64() <= subs[1]["ended_ms"].as_i64());
    assert!(subs[1]["started_ms"].as_i64() <= subs[0]["ended_ms"].as_i64());
    assert_eq!(
        (subs[0]["n"].as_u64(), subs[1]["n"].as_u64()),
        (Some(1), Some(2))
    );

    // A named agent not in the list is refused.
    let mut critic = start("toolu_9", "Criticise");
    critic.2["agent"] = json!("critic");
    fake.push(calls(&[critic]));
    fake.push(AnthropicMessage::text("Fine."));
    api.chat("lead", Some(&thread), "Ask the critic");
    let (err, out) = &results_of(&fake.message_requests().last().unwrap().json())[0];
    assert!(err);
    assert!(
        out["error"]
            .as_str()
            .unwrap()
            .contains("may not delegate to critic"),
        "{out}"
    );
}

#[test]
fn permissions_narrow_for_a_throwaway_and_meet_for_a_named_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread("lead");
    let mut narrowed = start("toolu_0", "Read only");
    narrowed.2["permissions"] = json!({
        "tools": ["get_note", "web_search"],
        "types": null,
        "confirm": null,
        "web": null,
        "browser": true,
        "run_code": null,
    });
    fake.push(calls(&[narrowed]));
    fake.push(AnthropicMessage::text("Read."));
    fake.push(AnthropicMessage::text("Done."));
    api.chat("lead", Some(&thread), "Go");
    let requests = fake.message_requests();
    assert_eq!(offered(&requests[1].json()), ["get_note"]);
    let (err, out) = &results_of(&requests[2].json())[0];
    assert!(!err, "{out}");
    let ignored = out["ignored_permissions"].as_array().unwrap();
    assert_eq!(ignored.len(), 2, "{out}");
    assert!(ignored[0].as_str().unwrap().contains("web_search"));
    assert!(ignored[1].as_str().unwrap().contains("Browser"));

    // The writer has query_notes and the lead search_notes: it gets only what both have, with
    // its own prompt, the task in its first message.
    fake.clear_requests();
    let mut named = start("toolu_1", "Write it up");
    named.2["agent"] = json!("writer");
    named.2["instructions"] = json!("Keep it short.");
    fake.push(calls(&[named]));
    fake.push(AnthropicMessage::text("Written."));
    fake.push(AnthropicMessage::text("Done."));
    api.chat("lead", Some(&thread), "Delegate");
    let sub = fake.message_requests()[1].json();
    assert_eq!(offered(&sub), ["get_note", "propose_create"]);
    let system = system_of(&sub);
    assert!(system.starts_with("You write notes."), "{system}");
    assert!(!system.contains("## Sub agent"));
    let first = sub["messages"][0]["content"][0]["text"].as_str().unwrap();
    assert!(first.contains("Lead hands you this task"), "{first}");
    assert!(first.contains("Write it up") && first.contains("Keep it short."));
}

#[test]
fn proposals_surface_in_the_parent_chat_and_changes_are_authored_by_the_sub_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);

    // The lead asks before note changes, and so does its throwaway sub agent: the proposal is
    // the parent's to apply.
    let thread = api.thread("lead");
    fake.push(calls(&[start("toolu_0", "Make a note")]));
    fake.push(AnthropicMessage::new().tool_use("toolu_s", "propose_create", item("From sub")));
    fake.push(AnthropicMessage::text("Proposed a note."));
    fake.push(AnthropicMessage::text("Over to you."));
    let events = api.chat("lead", Some(&thread), "Plan");
    let p = find(&events, "proposal").unwrap().clone();
    assert_eq!(p["subagent"], "toolu_0");
    assert_eq!(p["agent"], "lead");
    assert_eq!(p["sub"], 1);
    assert_eq!(p["thread"], thread.as_str());
    let sub_thread = p["sub_thread"].as_str().unwrap().to_owned();
    // The sub agent did not wait for the user, and said what it proposed.
    let (_, out) = &results_of(&fake.message_requests()[3].json())[0];
    assert!(
        out["proposed"][0]
            .as_str()
            .unwrap()
            .contains("waiting for the user"),
        "{out}"
    );
    // The parent thread and the session both hold it, pending.
    let parent = api.read_thread("lead", &thread).1;
    assert_eq!(parent["pending"], 1);
    let session = api.read_thread("lead", &sub_thread).1;
    assert_eq!(session["proposals"][0]["status"], "pending");

    let (status, done) = api.send("POST", "/api/ai/apply", Some(p));
    assert_eq!(status, StatusCode::OK, "{done}");
    let note = done["note_id"].as_str().unwrap();
    assert_eq!(
        api.author(note),
        json!({ "kind": "agent", "name": "lead/sub:1" })
    );
    let parent = api.read_thread("lead", &thread).1;
    assert_eq!(parent["proposals"][0]["status"], "applied");
    let session = api.read_thread("lead", &sub_thread).1;
    assert_eq!(session["proposals"][0]["status"], "applied");

    // With the switch off for both, the change is made at once, authored by the sub agent: a
    // throwaway as `<parent>/sub:<n>`, a named agent as itself.
    let thread = api.thread("free_lead");
    let mut named = start("toolu_1", "Write another");
    named.2["agent"] = json!("writer");
    fake.push(calls(&[start("toolu_0", "Make a note")]));
    fake.push(AnthropicMessage::new().tool_use("toolu_s", "propose_create", item("Made by sub")));
    fake.push(AnthropicMessage::text("Made it."));
    fake.push(calls(&[named]));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_w",
        "propose_create",
        item("Made by writer"),
    ));
    fake.push(AnthropicMessage::text("Wrote it."));
    fake.push(AnthropicMessage::text("Both made."));
    let events = api.chat("free_lead", Some(&thread), "Make notes");
    let applied: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "applied")
        .map(|(_, v)| v)
        .collect();
    assert_eq!(applied.len(), 2, "{events:?}");
    assert_eq!(applied[0]["subagent"], "toolu_0");
    assert_eq!(
        api.author(applied[0]["note_id"].as_str().unwrap()),
        json!({ "kind": "agent", "name": "free_lead/sub:1" })
    );
    assert_eq!(applied[1]["subagent"], "toolu_1");
    assert_eq!(
        api.author(applied[1]["note_id"].as_str().unwrap()),
        json!({ "kind": "agent", "name": "writer" })
    );
}

#[test]
fn stopping_the_parent_stops_its_sub_agents_and_stopping_one_answers_the_parent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let slow = || AnthropicMessage::text(&"slow ".repeat(60)).delay(Duration::from_millis(200));

    // Stop the sub agent alone: the parent goes on, told the user stopped it.
    let thread = api.thread("lead");
    fake.push(calls(&[start("toolu_0", "Take a while")]));
    fake.push(slow());
    fake.push(AnthropicMessage::text("Carried on."));
    let turn = api.start_turn("lead", &thread, "Go slowly");
    let events = api.turn_events(&turn, |(n, _)| n == "subagent");
    let sub_thread = events.last().unwrap().1["thread"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut sub_turn = String::new();
    api.wait("the sub agent's turn is listed", || {
        let t = api.read_thread("lead", &sub_thread).1;
        sub_turn = t["turn"].as_str().unwrap_or_default().to_owned();
        t["running"] == true && !sub_turn.is_empty()
    });
    // Its events read as any turn's.
    let first = api.turn_events(&sub_turn, |(n, _)| n == "text");
    assert_eq!(first.last().unwrap().0, "text");
    let (status, _) = api.send("DELETE", &format!("/api/ai/chat/turns/{sub_turn}"), None);
    assert_eq!(status, StatusCode::NO_CONTENT);
    let events = api.turn_events(&turn, |(n, _)| n == "done" || n == "error");
    assert_eq!(events.last().unwrap().0, "done", "{events:?}");
    let (err, out) = &results_of(&fake.message_requests().last().unwrap().json())[0];
    assert!(err);
    assert_eq!(out["outcome"], "stopped");
    assert!(out["status"].as_str().unwrap().contains("The user stopped"));
    api.wait("the session is free", || {
        api.read_thread("lead", &sub_thread).1["running"] == false
    });
    let t = api.read_thread("lead", &thread).1;
    assert_eq!(t["subagents"][0]["status"], "stopped");

    // Stop the parent: its sub agent stops with it.
    let thread = api.thread("lead");
    fake.push(calls(&[start("toolu_0", "Take a while")]));
    fake.push(slow());
    let turn = api.start_turn("lead", &thread, "Go slowly again");
    let events = api.turn_events(&turn, |(n, _)| n == "subagent");
    let sub_thread = events.last().unwrap().1["thread"]
        .as_str()
        .unwrap()
        .to_owned();
    api.wait("the sub agent runs", || {
        api.read_thread("lead", &sub_thread).1["running"] == true
    });
    let (status, _) = api.send("DELETE", &format!("/api/ai/chat/turns/{turn}"), None);
    assert_eq!(status, StatusCode::NO_CONTENT);
    api.wait("the sub agent stopped", || {
        api.read_thread("lead", &sub_thread).1["running"] == false
            && api.read_thread("lead", &thread).1["running"] == false
    });
    let t = api.read_thread("lead", &thread).1;
    assert_eq!(t["subagents"][0]["status"], "stopped", "{t}");
}

#[test]
fn sessions_are_listed_only_under_their_parent_and_go_with_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread("lead");
    fake.push(calls(&[start("toolu_0", "Look around")]));
    fake.push(AnthropicMessage::text("Looked."));
    fake.push(AnthropicMessage::text("Done."));
    api.chat("lead", Some(&thread), "Check");

    let parent = api.read_thread("lead", &thread).1;
    let s = &parent["subagents"][0];
    assert_eq!(s["call"], "toolu_0");
    assert_eq!(s["status"], "done");
    assert_eq!(s["task"], "Look around");
    assert_eq!(s["rounds"], 1);
    let sub_id = s["thread"].as_str().unwrap().to_owned();
    let sub = api.read_thread("lead", &sub_id).1;
    assert_eq!(sub["origin"], format!("subagent:{thread}"));
    assert_eq!(sub["parent"]["thread"], thread.as_str());
    assert_eq!(sub["parent"]["call"], "toolu_0");
    assert!(
        sub["parent"]["turn"]
            .as_str()
            .is_some_and(|t| !t.is_empty())
    );
    assert_eq!(sub["messages"].as_array().unwrap().len(), 2);
    let list = api.get("/api/agents/lead/threads?archived=true");
    let ids: Vec<&str> = list["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&thread.as_str()));
    assert!(!ids.contains(&sub_id.as_str()), "{list}");

    // Archived and taken out of the archive with its parent.
    let version = parent["version"].as_str().unwrap();
    api.send_ok(
        "POST",
        &format!("/api/notes/{thread}/archive?base_version={version}"),
        json!({}),
    );
    assert_eq!(api.read_thread("lead", &sub_id).1["archived"], true);
    let version = api.read_thread("lead", &thread).1["version"]
        .as_str()
        .unwrap()
        .to_owned();
    api.send_ok(
        "POST",
        &format!("/api/notes/{thread}/unarchive?base_version={version}"),
        json!({}),
    );
    assert_eq!(api.read_thread("lead", &sub_id).1["archived"], false);

    // A fork before the call has no sessions; truncating the parent past the call deletes them.
    let fork = api.send_ok(
        "POST",
        &format!("/api/agents/lead/threads/{thread}/fork"),
        json!({ "at": 0 }),
    );
    assert_eq!(fork["subagents"], json!([]));
    let cut = api.send_ok(
        "POST",
        &format!("/api/agents/lead/threads/{thread}/truncate"),
        json!({ "at": 0 }),
    );
    assert_eq!(cut["subagents"], json!([]));
    assert_eq!(api.read_thread("lead", &sub_id).0, StatusCode::NOT_FOUND);

    // Deleting the parent deletes its sessions.
    fake.push(calls(&[start("toolu_1", "Look again")]));
    fake.push(AnthropicMessage::text("Looked again."));
    fake.push(AnthropicMessage::text("Done."));
    api.chat("lead", Some(&thread), "Check again");
    let sub_id = api.read_thread("lead", &thread).1["subagents"][0]["thread"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(api.read_thread("lead", &sub_id).0, StatusCode::OK);
    let (status, _) = api.send(
        "DELETE",
        &format!("/api/agents/lead/threads/{thread}"),
        None,
    );
    assert!(status.is_success());
    assert_eq!(api.read_thread("lead", &sub_id).0, StatusCode::NOT_FOUND);

    // Each run is in the activity log, its turns under the session's thread.
    let page = api.get("/api/activity?category=ai&limit=200");
    let items = page["items"].as_array().unwrap();
    let of = |kind: &str| -> Vec<&Value> { items.iter().filter(|e| e["kind"] == kind).collect() };
    let started = of("ai.subagent.start");
    let ended = of("ai.subagent.end");
    assert_eq!((started.len(), ended.len()), (2, 2), "{page}");
    let end = ended
        .iter()
        .find(|e| e["detail"]["sub_thread"] == sub_id.as_str())
        .unwrap();
    assert_eq!(end["detail"]["parent_thread"], thread.as_str());
    assert_eq!(end["detail"]["outcome"], "done");
    assert_eq!(end["detail"]["rounds"], 1);
    assert_eq!(end["detail"]["throwaway"], true);
    assert!(end["detail"]["ms"].is_u64());
    assert!(
        of("ai.turn.start")
            .iter()
            .any(|e| e["detail"]["thread"] == sub_id.as_str()),
        "the sub agent's own turn logs with its thread"
    );
}

#[test]
fn sub_agents_are_granted_like_run_code() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // A manager that may change agents without asking, but has no sub agents.
    api.agent(
        "manager",
        "label = \"Manager\"\n\npermissions {\n  confirm {\n    agents = false\n  }\n}\n",
    );
    // A boss that has them and changes agents without asking.
    api.agent(
        "boss",
        "label = \"Boss\"\n\npermissions {\n  confirm {\n    agents = false\n  }\n\n  subagents {\n    allow = true\n    agents = [\"writer\"]\n  }\n}\n",
    );
    let grant = "label = \"Helper\"\n\npermissions {\n  subagents {\n    allow = true\n    agents = [\"writer\"]\n  }\n}\n";
    let propose = |agent: &str, name: &str, wcl: &str| {
        fake.push(AnthropicMessage::new().tool_use(
            "toolu_0",
            "propose_agent_change",
            json!({ "name": name, "wcl": wcl }),
        ));
        fake.push(AnthropicMessage::text("Done."));
        let events = api.chat(agent, None, "Do it.");
        let result = results_of(&fake.message_requests().last().unwrap().json()).remove(0);
        (events, result)
    };

    let (_, (err, out)) = propose("manager", "helper", grant);
    assert!(err, "{out}");
    assert!(
        out["error"].as_str().unwrap().contains("Sub agents"),
        "{out}"
    );
    // Nor may the boss add an agent it may not delegate to.
    let (_, (err, out)) = propose(
        "boss",
        "helper",
        "permissions {\n  subagents {\n    allow = true\n    agents = [\"critic\"]\n  }\n}\n",
    );
    assert!(err, "{out}");
    assert!(out["error"].as_str().unwrap().contains("critic"), "{out}");

    // The boss may propose what it has, but with its agents switch off the change still waits
    // for an Apply, which only an admin makes.
    let (events, (err, _)) = propose("boss", "helper", grant);
    assert!(!err);
    assert!(
        !events
            .iter()
            .any(|(n, v)| n == "applied" && v["op"] == "agent_create"),
        "{events:?}"
    );
    let p = find(&events, "agent_proposal").unwrap().clone();
    assert_eq!(p["grants_subagents"], true, "{p}");
    let write = api.token_of("phone", "write");
    let res = api.request_as(&write, "POST", "/api/ai/apply", Some(p.clone()));
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: Value = res.json().unwrap();
    assert_eq!(body["error"], "needs_admin");
    let (status, body) = api.send("POST", "/api/ai/apply", Some(p));
    assert_eq!(status, StatusCode::OK, "{body}");
    let helper = api.get("/api/agents/helper");
    assert_eq!(helper["permissions"]["subagents"]["allow"], true);
    assert_eq!(
        helper["permissions"]["subagents"]["agents"],
        json!(["writer"])
    );
}
