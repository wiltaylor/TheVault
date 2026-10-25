//! S1 with the fake Anthropic: agent memory (agents G21). The own memory tools `remember`,
//! `recall`, `update_memory` and `forget`, written as the agent and kept to its own memories;
//! the limits; the "Your memories" section of the system text and its cap; memories left out of
//! the note tools; and an unattended run offered the memory tools whatever its switches. The
//! Memory tab's routes; and the tools that reach another agent's memories, proposing under
//! `confirm.agents` or applying at once, refused for an agent that could not change the other
//! agent's file.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MEMO: &str = r#"label = "Memo"

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
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.send(
                "PUT",
                &format!("/api/secrets/{name}"),
                json!({ "value": value }),
            );
        }
        api
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Response {
        let url = self.server.url(path);
        let req = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            "DELETE" => self.client.delete(url),
            m => panic!("{m}"),
        }
        .bearer_auth(self.server.admin_token());
        match body {
            Some(b) => req.json(&b),
            None => req,
        }
        .send()
        .unwrap()
    }

    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let res = self.request(method, path, Some(body));
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        let res = self.request("GET", path, None);
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
            .bearer_auth(self.server.admin_token())
            .header("content-type", "text/plain")
            .body(wcl.to_owned())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    /// One chat turn of `agent` (the Assistant for `None`): the model calls `tool` with `input`
    /// and then answers. The tool's result as the model got it.
    fn call(&self, fake: &FakeAnthropic, agent: Option<&str>, tool: &str, input: Value) -> Value {
        fake.clear_requests();
        fake.push(AnthropicMessage::new().tool_use("toolu_m", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let mut body = json!({ "messages": [user("Go.")] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        let res = self.request("POST", "/api/ai/chat", Some(body));
        assert!(res.status().is_success());
        // Read the stream to its end, so the turn is over.
        let _ = res.text().unwrap();
        let r = fake.message_requests()[1].json();
        r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone()
    }

    /// One chat turn of `agent` (the Assistant for `None`) as `token`: the model calls `tool`
    /// with `input` and then answers. The turn's events.
    fn turn(
        &self,
        fake: &FakeAnthropic,
        token: &str,
        agent: Option<&str>,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        fake.clear_requests();
        fake.push(AnthropicMessage::new().tool_use("toolu_c", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let mut body = json!({ "messages": [user("Go.")] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .unwrap();
        assert!(res.status().is_success());
        events(&res.text().unwrap())
    }

    /// The tool result of the last turn as the model got it.
    fn result(&self, fake: &FakeAnthropic) -> Value {
        let r = fake.message_requests()[1].json();
        r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone()
    }

    /// `POST /api/ai/apply` as `token`: the status and body.
    fn apply_as(&self, token: &str, change: &Value) -> (u16, Value) {
        let res = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .bearer_auth(token)
            .json(change)
            .send()
            .unwrap();
        (res.status().as_u16(), res.json().unwrap_or(Value::Null))
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let v = self.send(
            "POST",
            "/api/tokens",
            json!({ "name": name, "scope": scope }),
        );
        v["secret"].as_str().unwrap().to_owned()
    }

    /// The memories `GET /api/agents/{agent}/memories` lists, by title.
    fn titles(&self, agent: &str) -> Vec<String> {
        self.get(&format!("/api/agents/{agent}/memories"))["memories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["title"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Memory notes made directly, in one batch, for agent `agent`.
    fn memories(&self, agent: &str, titles: &[String], text: &str) -> Vec<String> {
        let ops: Vec<Value> = titles
            .iter()
            .map(|t| {
                json!({ "op": "create", "type": "agent_memory", "note": {
                    "fields": { "title": t, "agent": agent },
                    "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
                        { "kind": "paragraph", "fields": { "text": text } },
                    ] }],
                } })
            })
            .collect();
        let v = self.send("POST", "/api/batch", json!({ "ops": ops }));
        assert_eq!(v["committed"], true, "{v}");
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect()
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

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

/// The events of a server-sent event stream.
fn events(text: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    for line in text.lines().chain(std::iter::once("")) {
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

fn find<'a>(events: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    events.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// A tool result's content as JSON.
fn content(result: &Value) -> Value {
    serde_json::from_str(result["content"].as_str().unwrap()).unwrap()
}

fn tool_names(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

fn system(fake: &FakeAnthropic, request: usize) -> String {
    fake.message_requests()[request].json()["system"]
        .as_str()
        .unwrap()
        .to_owned()
}

const CLERK: &str = "label = \"Clerk\"\n";

#[test]
fn an_agent_remembers_recalls_updates_and_forgets_its_own_memories() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);

    // remember: a memory note of the agent's, written by it.
    let r = api.call(
        &fake,
        Some("clerk"),
        "remember",
        json!({ "title": "Tea", "text": "Earl Grey, *no* milk.\n\nNever before noon." }),
    );
    assert_eq!(r["is_error"], false, "{r}");
    let tea = content(&r)["id"].as_str().unwrap().to_owned();
    let note = api.get(&format!("/api/notes/{tea}"));
    assert_eq!(note["type"], "agent_memory");
    assert_eq!(note["note"]["fields"]["agent"], "clerk");
    assert_eq!(note["note"]["fields"]["title"], "Tea");
    assert_eq!(
        api.author(&tea),
        json!({ "kind": "agent", "name": "clerk" })
    );
    let r = api.call(
        &fake,
        Some("clerk"),
        "remember",
        json!({ "title": "Garden", "text": "Water the roses on Sundays." }),
    );
    let garden = content(&r)["id"].as_str().unwrap().to_owned();

    // recall: all, newest first, with the full text; then by query.
    let r = api.call(
        &fake,
        Some("clerk"),
        "recall",
        json!({ "query": "", "limit": null }),
    );
    let got = content(&r);
    let ids: Vec<&str> = got["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [garden.as_str(), tea.as_str()], "{got}");
    assert_eq!(
        got["memories"][1]["text"],
        "Earl Grey, *no* milk.\n\nNever before noon."
    );
    let r = api.call(
        &fake,
        Some("clerk"),
        "recall",
        json!({ "query": "MILK grey", "limit": 5 }),
    );
    let got = content(&r);
    assert_eq!(got["matched"], 1);
    assert_eq!(got["memories"][0]["id"], tea.as_str());
    let r = api.call(
        &fake,
        Some("clerk"),
        "recall",
        json!({ "query": "", "limit": 1 }),
    );
    assert_eq!(content(&r)["memories"].as_array().unwrap().len(), 1);

    // update_memory: null keeps a part.
    let r = api.call(
        &fake,
        Some("clerk"),
        "update_memory",
        json!({ "id": tea, "title": null, "text": "Green tea now." }),
    );
    assert_eq!(r["is_error"], false, "{r}");
    let r = api.call(
        &fake,
        Some("clerk"),
        "recall",
        json!({ "query": "green", "limit": null }),
    );
    let got = content(&r);
    assert_eq!(got["memories"][0]["title"], "Tea");
    assert_eq!(got["memories"][0]["text"], "Green tea now.");
    assert_eq!(
        api.author(&tea),
        json!({ "kind": "agent", "name": "clerk" })
    );

    // Another agent cannot touch the clerk's memories, nor see them.
    let r = api.call(
        &fake,
        None,
        "update_memory",
        json!({ "id": tea, "title": "Mine", "text": null }),
    );
    assert_eq!(r["is_error"], true, "{r}");
    assert!(
        r["content"].as_str().unwrap().contains("is not yours"),
        "{r}"
    );
    let r = api.call(&fake, None, "forget", json!({ "id": tea }));
    assert_eq!(r["is_error"], true, "{r}");
    let r = api.call(&fake, None, "recall", json!({ "query": "", "limit": null }));
    assert_eq!(content(&r)["memories"], json!([]));
    // A note that is not a memory is not one to forget.
    let memo = api.send(
        "POST",
        "/api/notes?type=memo",
        json!({ "note": { "fields": { "title": "Plain" } } }),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = api.call(&fake, Some("clerk"), "forget", json!({ "id": memo }));
    assert_eq!(r["is_error"], true, "{r}");

    // forget: the memory goes to the trash, deleted by the agent.
    let r = api.call(&fake, Some("clerk"), "forget", json!({ "id": garden }));
    assert_eq!(r["is_error"], false, "{r}");
    let trash = api.get("/api/trash");
    let entry = trash["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == garden.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        entry["deleted_by"],
        json!({ "kind": "agent", "name": "clerk" })
    );
    let r = api.call(
        &fake,
        Some("clerk"),
        "recall",
        json!({ "query": "", "limit": null }),
    );
    assert_eq!(content(&r)["held"], 1);
}

#[test]
fn the_limits_hold_for_titles_texts_and_the_number_of_memories() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);

    let r = api.call(
        &fake,
        Some("clerk"),
        "remember",
        json!({ "title": "Long", "text": "x".repeat(8_001) }),
    );
    assert_eq!(r["is_error"], true, "{r}");
    assert!(r["content"].as_str().unwrap().contains("8000"), "{r}");
    let r = api.call(
        &fake,
        Some("clerk"),
        "remember",
        json!({ "title": "t".repeat(121), "text": "Short." }),
    );
    assert_eq!(r["is_error"], true, "{r}");
    assert!(r["content"].as_str().unwrap().contains("120"), "{r}");
    let r = api.call(
        &fake,
        Some("clerk"),
        "remember",
        json!({ "title": "Exactly", "text": "y".repeat(8_000) }),
    );
    assert_eq!(r["is_error"], false, "{r}");
    let id = content(&r)["id"].as_str().unwrap().to_owned();
    let r = api.call(
        &fake,
        Some("clerk"),
        "update_memory",
        json!({ "id": id, "title": null, "text": "z".repeat(8_001) }),
    );
    assert_eq!(r["is_error"], true, "{r}");

    // 500 memories in all: the 501st is refused, asking to forget or merge some.
    let titles: Vec<String> = (1..500).map(|i| format!("Fact {i}")).collect();
    api.memories("clerk", &titles, "Something.");
    let r = api.call(
        &fake,
        Some("clerk"),
        "remember",
        json!({ "title": "One more", "text": "Too many." }),
    );
    assert_eq!(r["is_error"], true, "{r}");
    let message = r["content"].as_str().unwrap();
    assert!(message.contains("500"), "{r}");
    assert!(message.contains("Forget some"), "{r}");
    // Another agent's count is its own.
    let r = api.call(
        &fake,
        None,
        "remember",
        json!({ "title": "Mine", "text": "Fine." }),
    );
    assert_eq!(r["is_error"], false, "{r}");
}

#[test]
fn the_system_text_lists_the_agents_memories_after_its_prompt_capped() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "clerk",
        "label = \"Clerk\"\nsystem_prompt = \"You keep the books.\"\n",
    );
    api.agent("other", "label = \"Other\"\n");

    // No memories, no section.
    fake.push(AnthropicMessage::text("Hi."));
    api.send(
        "POST",
        "/api/ai/chat",
        json!({ "agent": "clerk", "messages": [user("Hi")] }),
    );
    assert!(!system(&fake, 0).contains("Your memories"));

    let titles: Vec<String> = (1..=60).map(|i| format!("Fact {i:02}")).collect();
    let text = format!("{} end", "word ".repeat(60));
    let ids = api.memories("clerk", &titles, &text);
    api.memories("other", &["Not the clerk's".to_owned()], "Elsewhere.");

    fake.clear_requests();
    fake.push(AnthropicMessage::text("Hi."));
    let res = api.request(
        "POST",
        "/api/ai/chat",
        Some(json!({ "agent": "clerk", "messages": [user("Hi")] })),
    );
    let _ = res.text().unwrap();
    let s = system(&fake, 0);
    assert!(s.starts_with("You keep the books.\n"), "{s}");
    let section = &s[s.find("## Your memories").expect("the section")..];
    let lines: Vec<&str> = section
        .lines()
        .filter(|l| l.starts_with("- Fact "))
        .collect();
    // Each line holds the first 200 characters of its text, and the lines stop at 8000.
    let first = lines[0];
    // One batch made them all at once, so the newest first is the one with the greatest id.
    let newest = ids.iter().max().unwrap();
    assert!(first.contains(&format!("({newest})")), "{first}");
    assert!(first.ends_with('…'), "{first}");
    let shown: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
    assert!(shown <= 8_000, "{shown}");
    assert!(lines.len() < 60, "{}", lines.len());
    let more = 60 - lines.len();
    assert!(
        section.contains(&format!(
            "- {more} more memories are not shown here: `recall` reads them."
        )),
        "{section}"
    );
    assert!(!section.contains("Not the clerk's"), "{section}");
}

#[test]
fn memories_are_left_out_of_the_note_tools_unless_the_agent_names_the_type() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.agent(
        "keeper",
        "label = \"Keeper\"\n\npermissions {\n  types = [\"agent_memory\"]\n}\n",
    );
    let id = api.memories("clerk", &["Zebra crossing".to_owned()], "Stripes.")[0].clone();

    let r = api.call(
        &fake,
        Some("clerk"),
        "search_notes",
        json!({ "text": "zebra", "limit": 10 }),
    );
    assert!(!r["content"].as_str().unwrap().contains(&id), "{r}");
    let r = api.call(
        &fake,
        Some("clerk"),
        "query_notes",
        json!({ "query": "from agent_memory" }),
    );
    assert_eq!(content(&r)["rows"], json!([]), "{r}");
    let r = api.call(&fake, Some("clerk"), "get_note", json!({ "id": id }));
    assert_eq!(r["is_error"], true, "{r}");
    assert!(
        r["content"]
            .as_str()
            .unwrap()
            .contains("This agent may not read agent_memory notes."),
        "{r}"
    );
    // ...while the clerk's own recall reads it.
    let r = api.call(
        &fake,
        Some("clerk"),
        "recall",
        json!({ "query": "zebra", "limit": null }),
    );
    assert_eq!(content(&r)["memories"][0]["id"], id.as_str());

    let r = api.call(
        &fake,
        Some("keeper"),
        "search_notes",
        json!({ "text": "zebra", "limit": 10 }),
    );
    assert!(r["content"].as_str().unwrap().contains(&id), "{r}");
}

#[test]
fn an_unattended_run_is_offered_the_memory_tools_and_remembers_as_the_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // Every confirmation switch on: no change tool is offered, the memory tools are.
    api.agent(
        "clerk",
        "label = \"Clerk\"\n\ntrigger any {\n  on     = [\"create\"]\n  type   = \"memo\"\n  prompt = \"Note what came in.\"\n}\n",
    );
    let tools = api.get("/api/agents/clerk/runs")["tools"].clone();
    let tools: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    for t in ["remember", "recall", "update_memory", "forget"] {
        assert!(tools.contains(&t), "{t}: {tools:?}");
    }
    assert!(!tools.contains(&"propose_edit"), "{tools:?}");

    fake.push(AnthropicMessage::new().tool_use(
        "toolu_r",
        "remember",
        json!({ "title": "Memos", "text": "A memo came in." }),
    ));
    fake.push(AnthropicMessage::text("Noted."));
    api.send(
        "POST",
        "/api/notes?type=memo",
        json!({ "note": { "fields": { "title": "Roof" } } }),
    );
    api.send("POST", "/api/test/clock", json!({ "advance": "PT31S" }));
    let offered = tool_names(&fake, 0);
    assert_eq!(offered, tools, "the run is offered what the route says");
    let result = fake.message_requests()[1].json()["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["content"][0]
        .clone();
    assert_eq!(result["is_error"], false, "{result}");
    let id = content(&result)["id"].as_str().unwrap().to_owned();
    assert_eq!(api.author(&id), json!({ "kind": "agent", "name": "clerk" }));
    // The agent's own write fires no further run: the trigger names only memos anyway, and
    // the memory is the agent's.
    assert_eq!(
        api.get("/api/agents/clerk/runs")["triggers"][0]["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // An agent whose tools list leaves the memory tools out is not offered them.
    api.agent(
        "reader",
        "label = \"Reader\"\n\npermissions {\n  tools = [\"get_note\", \"recall\"]\n}\n\ntrigger any {\n  on = [\"create\"]\n  prompt = \"Read it.\"\n}\n",
    );
    let tools = api.get("/api/agents/reader/runs")["tools"].clone();
    assert_eq!(tools, json!(["get_note", "recall"]));
}

#[test]
fn the_memory_routes_list_search_add_edit_and_delete_as_the_caller() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);

    // Add: authored as the caller, refused past the limits.
    let res = api.request(
        "POST",
        "/api/agents/clerk/memories",
        Some(json!({ "title": "Tea", "text": "Earl Grey, no milk." })),
    );
    assert_eq!(res.status().as_u16(), 201);
    let tea: Value = res.json().unwrap();
    assert_eq!(tea["agent"], "clerk");
    assert_eq!(tea["text"], "Earl Grey, no milk.");
    let tea_id = tea["id"].as_str().unwrap().to_owned();
    assert_eq!(
        api.author(&tea_id),
        json!({ "kind": "token", "name": "admin" })
    );
    let res = api.request(
        "POST",
        "/api/agents/clerk/memories",
        Some(json!({ "title": "x".repeat(121), "text": "Long." })),
    );
    assert_eq!(res.status().as_u16(), 400);
    assert_eq!(res.json::<Value>().unwrap()["error"], "invalid_memory");
    api.send(
        "POST",
        "/api/agents/clerk/memories",
        json!({ "title": "Garden", "text": "Water on Sundays." }),
    );

    // List: newest first, held counts all; search keeps every word's matches.
    assert_eq!(api.titles("clerk"), ["Garden", "Tea"]);
    let found = api.get("/api/agents/clerk/memories?query=MILK%20tea");
    assert_eq!(found["memories"].as_array().unwrap().len(), 1);
    assert_eq!(found["memories"][0]["title"], "Tea");
    assert_eq!(found["held"], 2);
    assert_eq!(api.titles("helper"), Vec::<String>::new());

    // Edit: title and text together; a stale base is a conflict.
    let changed = api.send(
        "PATCH",
        &format!("/api/agents/clerk/memories/{tea_id}"),
        json!({ "title": "Tea order", "text": "Green tea.", "base_version": tea["version"] }),
    );
    assert_eq!(changed["title"], "Tea order");
    assert_eq!(changed["text"], "Green tea.");
    let res = api.request(
        "PATCH",
        &format!("/api/agents/clerk/memories/{tea_id}"),
        Some(json!({ "text": "Coffee.", "base_version": tea["version"] })),
    );
    assert_eq!(res.status().as_u16(), 409);
    // Another agent's path is a 404.
    let res = api.request(
        "PATCH",
        &format!("/api/agents/helper/memories/{tea_id}"),
        Some(json!({ "text": "Coffee." })),
    );
    assert_eq!(res.status().as_u16(), 404);

    // Delete: to the trash, by the caller.
    let res = api.request(
        "DELETE",
        &format!("/api/agents/helper/memories/{tea_id}"),
        None,
    );
    assert_eq!(res.status().as_u16(), 404);
    api.send(
        "DELETE",
        &format!("/api/agents/clerk/memories/{tea_id}"),
        json!({}),
    );
    assert_eq!(api.titles("clerk"), ["Garden"]);
    let trash = api.get("/api/trash");
    assert!(
        trash["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == tea_id.as_str())
    );

    // A read token lists; a write token changes.
    let read = api.token_of("reader", "read");
    let write = api.token_of("phone", "write");
    let as_token = |token: &str, post: bool| {
        let url = api.server.url("/api/agents/clerk/memories");
        let req = if post {
            api.client
                .post(url)
                .json(&json!({ "title": "Phone", "text": "From the app." }))
        } else {
            api.client.get(url)
        };
        req.bearer_auth(token).send().unwrap().status().as_u16()
    };
    assert_eq!(as_token(&read, false), 200);
    assert_eq!(as_token(&read, true), 403);
    assert_eq!(as_token(&write, true), 201);
}

/// An agent that manages agents within narrow permissions.
const WARDEN: &str = r#"label = "Warden"

permissions {
  tools = ["get_note", "propose_agent_change", "list_agent_memories", "get_agent_memory", "propose_agent_memory_change", "propose_agent_memory_delete"]
}
"#;

/// An agent within the warden's permissions.
const HELPER: &str = r#"label = "Helper"

permissions {
  tools = ["get_note"]
}
"#;

#[test]
fn another_agents_memories_are_read_and_changed_through_proposals() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    let admin = api.server.admin_token().to_owned();
    let tea = api.memories("clerk", &["Tea".to_owned()], "Earl Grey.")[0].clone();

    // The Assistant has every tool, so it reaches the clerk's memories.
    let events = api.turn(
        &fake,
        &admin,
        None,
        "list_agent_memories",
        json!({ "agent": "clerk", "query": "" }),
    );
    assert!(find(&events, "done").is_some(), "{events:?}");
    let tools = tool_names(&fake, 0);
    for t in [
        "list_agent_memories",
        "get_agent_memory",
        "propose_agent_memory_change",
        "propose_agent_memory_delete",
    ] {
        assert!(tools.contains(&t.to_owned()), "{t} in {tools:?}");
    }
    let r = api.result(&fake);
    assert_eq!(r["is_error"], false, "{r}");
    let listed = content(&r);
    assert_eq!(listed["memories"][0]["id"], tea.as_str(), "{listed}");
    assert_eq!(listed["memories"][0]["text"], "Earl Grey.");
    api.turn(
        &fake,
        &admin,
        None,
        "get_agent_memory",
        json!({ "id": tea }),
    );
    let got = content(&api.result(&fake));
    assert_eq!(got["agent"], "clerk");
    assert_eq!(got["title"], "Tea");

    // Create: proposed, not written; applied, written as the proposing agent.
    let events = api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_change",
        json!({ "agent": "clerk", "id": null, "title": "Garden", "text": "Water on Sundays." }),
    );
    let p = find(&events, "agent_memory_proposal")
        .unwrap_or_else(|| panic!("{events:?}"))
        .clone();
    assert_eq!(p["kind"], "agent_memory");
    assert_eq!(p["op"], "create");
    assert_eq!(p["memory_agent"], "clerk");
    assert_eq!(p["proposer"], "assistant");
    assert_eq!(api.titles("clerk"), ["Tea"], "nothing written yet");
    // Memories are notes: a write token applies them.
    let write = api.token_of("phone", "write");
    let (status, done) = api.apply_as(&write, &p);
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["op"], "agent_memory_create");
    assert_eq!(done["memory_agent"], "clerk");
    let garden = done["memory_id"].as_str().unwrap().to_owned();
    assert_eq!(
        api.author(&garden),
        json!({ "kind": "token", "name": "phone" }),
        "the Assistant's proposals are applied as the caller"
    );
    assert_eq!(api.titles("clerk"), ["Garden", "Tea"]);

    // Change: null keeps the title; a memory changed since is stale.
    let events = api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_change",
        json!({ "agent": "clerk", "id": tea, "title": null, "text": "Green tea." }),
    );
    let p = find(&events, "agent_memory_proposal").unwrap().clone();
    assert_eq!(p["op"], "change");
    assert_eq!(p["title"], "Tea");
    assert_eq!(p["text"], "Green tea.");
    assert_eq!(p["before"], json!({ "title": "Tea", "text": "Earl Grey." }));
    api.send(
        "PATCH",
        &format!("/api/agents/clerk/memories/{tea}"),
        json!({ "text": "Oolong." }),
    );
    let (status, body) = api.apply_as(&admin, &p);
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "stale");
    let events = api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_change",
        json!({ "agent": "clerk", "id": tea, "title": null, "text": "Green tea." }),
    );
    let p = find(&events, "agent_memory_proposal").unwrap().clone();
    let (status, done) = api.apply_as(&admin, &p);
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["op"], "agent_memory_change");
    let got = api.get("/api/agents/clerk/memories?query=green");
    assert_eq!(got["memories"][0]["id"], tea.as_str());

    // A memory named under the wrong agent is refused.
    api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_change",
        json!({ "agent": "helper", "id": tea, "title": "X", "text": null }),
    );
    assert_eq!(api.result(&fake)["is_error"], true);

    // Delete: to the trash once applied.
    let events = api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_delete",
        json!({ "id": tea }),
    );
    let p = find(&events, "agent_memory_proposal").unwrap().clone();
    assert_eq!(p["op"], "delete");
    assert_eq!(api.titles("clerk").len(), 2, "nothing deleted yet");
    let (status, done) = api.apply_as(&admin, &p);
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["op"], "agent_memory_delete");
    assert_eq!(api.titles("clerk"), ["Garden"]);

    // Its own memories are not for these tools.
    let mine = api.memories("assistant", &["Mine".to_owned()], "Mine.")[0].clone();
    api.turn(
        &fake,
        &admin,
        None,
        "list_agent_memories",
        json!({ "agent": "assistant", "query": "" }),
    );
    let r = api.result(&fake);
    assert_eq!(r["is_error"], true, "{r}");
    assert!(r["content"].as_str().unwrap().contains("your own"), "{r}");
    api.turn(
        &fake,
        &admin,
        None,
        "get_agent_memory",
        json!({ "id": mine }),
    );
    assert_eq!(api.result(&fake)["is_error"], true);
}

#[test]
fn with_the_agents_switch_off_memory_changes_apply_at_once() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.send(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "confirm": { "agents": false } }),
    );
    let admin = api.server.admin_token().to_owned();
    let events = api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_change",
        json!({ "agent": "clerk", "id": null, "title": "Garden", "text": "Water on Sundays." }),
    );
    assert!(
        find(&events, "agent_memory_proposal").is_none(),
        "{events:?}"
    );
    let applied = find(&events, "applied").unwrap();
    assert_eq!(applied["op"], "agent_memory_create");
    assert_eq!(api.titles("clerk"), ["Garden"]);
    let r = content(&api.result(&fake));
    assert_eq!(r["applied"], true, "{r}");
    let id = applied["memory_id"].as_str().unwrap().to_owned();

    let events = api.turn(
        &fake,
        &admin,
        None,
        "propose_agent_memory_delete",
        json!({ "id": id }),
    );
    assert_eq!(
        find(&events, "applied").unwrap()["op"],
        "agent_memory_delete"
    );
    assert_eq!(api.titles("clerk"), Vec::<String>::new());
}

#[test]
fn a_weaker_agent_cannot_reach_a_stronger_agents_memories() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let admin = api.server.admin_token().to_owned();
    api.agent("clerk", CLERK);
    api.agent("warden", WARDEN);
    api.agent("helper", HELPER);
    let tea = api.memories("clerk", &["Tea".to_owned()], "Earl Grey.")[0].clone();
    let help = api.memories("helper", &["Help".to_owned()], "Be kind.")[0].clone();

    // The clerk has every tool, which the warden lacks: every tool is refused.
    for (tool, input) in [
        (
            "list_agent_memories",
            json!({ "agent": "clerk", "query": "" }),
        ),
        ("get_agent_memory", json!({ "id": tea })),
        (
            "propose_agent_memory_change",
            json!({ "agent": "clerk", "id": tea, "title": null, "text": "Coffee." }),
        ),
        ("propose_agent_memory_delete", json!({ "id": tea })),
    ] {
        let events = api.turn(&fake, &admin, Some("warden"), tool, input);
        assert!(find(&events, "agent_memory_proposal").is_none(), "{tool}");
        let r = api.result(&fake);
        assert_eq!(r["is_error"], true, "{tool}: {r}");
        assert!(
            r["content"].as_str().unwrap().contains("out of reach"),
            "{tool}: {r}"
        );
    }

    // The helper is within the warden's permissions.
    api.turn(
        &fake,
        &admin,
        Some("warden"),
        "list_agent_memories",
        json!({ "agent": "helper", "query": "" }),
    );
    let r = api.result(&fake);
    assert_eq!(r["is_error"], false, "{r}");
    assert_eq!(content(&r)["memories"][0]["id"], help.as_str());
    let events = api.turn(
        &fake,
        &admin,
        Some("warden"),
        "propose_agent_memory_delete",
        json!({ "id": help }),
    );
    let p = find(&events, "agent_memory_proposal").unwrap().clone();
    assert_eq!(p["proposer"], "warden");

    // Apply checks the proposer again as it stands: the helper given every tool since is out
    // of the warden's reach.
    api.agent("helper", "label = \"Helper\"\n");
    let (status, body) = api.apply_as(&admin, &p);
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"], "not_permitted");
    assert_eq!(api.titles("helper"), ["Help"]);

    // An agent without propose_agent_change reaches no other agent's memories.
    api.agent(
        "reader",
        "label = \"Reader\"\n\npermissions {\n  tools = [\"list_agent_memories\"]\n}\n",
    );
    api.agent("helper", HELPER);
    api.turn(
        &fake,
        &admin,
        Some("reader"),
        "list_agent_memories",
        json!({ "agent": "helper", "query": "" }),
    );
    assert_eq!(api.result(&fake)["is_error"], true);

    // Unattended with the switch on, the warden is offered the reads, not the proposals.
    let tools = api.get("/api/agents/warden/runs")["tools"].clone();
    let tools: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(tools.contains(&"list_agent_memories"), "{tools:?}");
    assert!(tools.contains(&"get_agent_memory"), "{tools:?}");
    assert!(!tools.contains(&"propose_agent_memory_change"), "{tools:?}");
    assert!(!tools.contains(&"propose_agent_memory_delete"), "{tools:?}");
    api.agent(
        "warden",
        &WARDEN.replace("]\n}", "]\n\n  confirm {\n    agents = false\n  }\n}"),
    );
    let tools = api.get("/api/agents/warden/runs")["tools"].clone();
    assert!(
        tools
            .as_array()
            .unwrap()
            .contains(&json!("propose_agent_memory_change")),
        "{tools}"
    );
}
