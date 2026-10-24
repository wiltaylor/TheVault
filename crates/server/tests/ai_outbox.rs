//! S1 with the fake Anthropic provider: the chat's Outbox tools (outbox X9, X11). `draft_email`,
//! `draft_sms` and `draft_event` create Outbox items as the agent, as drafts, or ready only
//! for an agent with `send_without_review`; a person note id resolves to the person's email or
//! phone; the tools need a caller with `write`; they are ordinary tool names in an agent's tools
//! list and are offered to unattended runs; the agent cannot then make its own item ready, even
//! with note confirmation off; and an agent cannot give another agent the bypass it lacks.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic, TestServer};

const MODEL: &str = "claude-opus-5";

/// An agent that drafts, with note confirmation off, and may manage agents.
const CLERK: &str = r#"label = "Clerk"

permissions {
  tools = ["get_note", "propose_edit", "draft_email", "draft_sms", "draft_event", "propose_agent_change"]

  confirm {
    notes = false
    agents = false
  }
}
"#;

/// An agent whose drafts send without review.
const SENDER: &str = r#"label = "Sender"

permissions {
  tools = ["draft_email", "propose_agent_change"]

  confirm {
    agents = false
  }

  outbox {
    send_without_review = true
  }
}
"#;

/// An agent without the Outbox tools.
const READER: &str = r#"label = "Reader"

permissions {
  tools = ["get_note"]
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start();
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

    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let url = self.server.url(path);
        let req = match method {
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            m => panic!("{m}"),
        };
        let res = req
            .bearer_auth(self.server.admin_token())
            .json(&body)
            .send()
            .unwrap();
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        let res = self
            .client
            .get(self.server.url(path))
            .bearer_auth(self.server.admin_token())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "GET {path}");
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

    fn agent_exists(&self, name: &str) -> bool {
        self.client
            .get(self.server.url(&format!("/api/agents/{name}")))
            .bearer_auth(self.server.admin_token())
            .send()
            .unwrap()
            .status()
            .is_success()
    }

    fn note(&self, type_name: &str, fields: Value) -> String {
        let v = self.send(
            "POST",
            &format!("/api/notes?type={type_name}"),
            json!({ "note": { "fields": fields } }),
        );
        v["id"].as_str().unwrap().to_owned()
    }

    fn fields(&self, id: &str) -> Value {
        self.get(&format!("/api/notes/{id}"))["note"]["fields"].clone()
    }

    fn note_json(&self, id: &str) -> Value {
        self.get(&format!("/api/notes/{id}"))
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/tokens"))
            .bearer_auth(self.server.admin_token())
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        v["secret"].as_str().unwrap().to_owned()
    }

    /// One turn as `token`, run by `agent` (Assistant when `None`): its events.
    fn chat_as(&self, token: &str, agent: Option<&str>) -> Vec<(String, Value)> {
        let mut body = json!({ "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Do it." }] }] });
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
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    /// One tool call by `agent` as the admin, then an answer: the turn's events.
    fn one(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        self.one_as(self.server.admin_token(), fake, agent, tool, input)
    }

    fn one_as(
        &self,
        token: &str,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(token, agent)
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

fn last<'a>(events: &'a [(String, Value)], name: &str) -> &'a Value {
    &events
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
        .1
}

/// The tool result the model got back in the last request, as JSON, after checking its flag.
fn last_result(fake: &FakeAnthropic, is_error: bool) -> Value {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    let result = r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone();
    assert_eq!(result["is_error"], is_error, "{result}");
    serde_json::from_str(result["content"].as_str().unwrap()).unwrap()
}

/// The names of the tools request `request` offered.
fn tools_in(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

fn email_input(to: Value) -> Value {
    json!({
        "to": to,
        "cc": [],
        "bcc": [],
        "subject": "Lunch on Friday?",
        "body": "Hi Ann,\n\nAre you free for lunch on Friday?",
        "attachments": [],
        "in_reply_to": "",
    })
}

#[test]
fn the_draft_tools_create_drafts_resolving_people() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let ann = api.note(
        "person",
        json!({ "title": "Ann Lee", "emails": ["ann@example.com"], "phones": ["+61 400 000 001"] }),
    );

    // Assistant has every tool, so the draft tools are offered.
    let events = api.one(&fake, None, "draft_email", email_input(json!([ann])));
    let tools = tools_in(&fake, 0);
    for t in ["draft_email", "draft_sms", "draft_event"] {
        assert!(tools.contains(&t.to_owned()), "{t}: {tools:?}");
    }
    let r = last_result(&fake, false);
    assert_eq!(r["status"], "draft", "{r}");
    assert!(
        r["message"].as_str().unwrap().contains("has not been sent"),
        "{r}"
    );
    let id = r["id"].as_str().unwrap();
    let f = api.fields(id);
    assert_eq!(f["kind"], "email");
    assert_eq!(f["status"], "draft");
    assert_eq!(f["drafted_by"], "assistant");
    assert_eq!(f["to"], json!(["ann@example.com"]));
    assert_eq!(f["title"], "Lunch on Friday?");
    let blocks = api.note_json(id)["note"]["blocks"].clone();
    assert_eq!(blocks[0]["labels"][0], "body", "{blocks}");
    assert_eq!(blocks[0]["blocks"].as_array().unwrap().len(), 2, "{blocks}");
    // The chat's card reads the step's summary.
    let summary = &last(&events, "tool_result")["summary"];
    assert_eq!(summary["draft"]["note_id"], id, "{summary}");
    assert_eq!(summary["draft"]["kind"], "email");
    assert_eq!(summary["draft"]["status"], "draft");
    assert_eq!(summary["draft"]["to"], json!(["Ann Lee"]));

    // An SMS to a person goes to their first phone number.
    api.one(
        &fake,
        None,
        "draft_sms",
        json!({ "to": ann, "body": "Running late.", "device": "" }),
    );
    let r = last_result(&fake, false);
    let f = api.fields(r["id"].as_str().unwrap());
    assert_eq!(f["kind"], "sms");
    assert_eq!(f["status"], "draft");
    assert_eq!(f["phone"], "+61 400 000 001");
    assert_eq!(f["title"], "SMS to Ann Lee");

    // A new event, and a change to an existing one.
    let invite = json!({
        "event": "",
        "title": "Planning",
        "when": { "start": "2026-11-02T09:30:00+11:00", "end": "2026-11-02T10:00:00+11:00" },
        "location": "Room 1",
        "attendees": [ann, "bob@example.com"],
        "description": "The plan for next year.",
        "calendar": "",
    });
    api.one(&fake, None, "draft_event", invite.clone());
    let r = last_result(&fake, false);
    let f = api.fields(r["id"].as_str().unwrap());
    assert_eq!(f["kind"], "event");
    assert_eq!(f["status"], "draft");
    assert_eq!(f["title"], "Planning");
    assert_eq!(
        f["attendees_raw"],
        json!(["ann@example.com", "bob@example.com"])
    );
    assert_eq!(f["when"]["start"], "2026-11-02T09:30:00+11:00");
    assert!(f.get("event").is_none_or(Value::is_null), "{f}");

    let standup = api.note(
        "event",
        json!({ "title": "Standup", "when": { "start": "2026-11-03", "end": "2026-11-03" } }),
    );
    let mut change = invite.clone();
    change["event"] = json!(standup);
    change["title"] = json!("");
    change["when"] = json!({ "start": "", "end": "" });
    change["attendees"] = json!([]);
    change["description"] = json!("");
    api.one(&fake, None, "draft_event", change);
    let r = last_result(&fake, false);
    let f = api.fields(r["id"].as_str().unwrap());
    assert_eq!(f["event"], standup.as_str());
    assert_eq!(f["title"], "Standup");
    assert_eq!(f["location"], "Room 1");
    assert!(f.get("when").is_none_or(Value::is_null), "{f}");

    // Mistakes come back as errors and draft nothing.
    for (tool, input, says) in [
        ("draft_email", email_input(json!([])), "at least one"),
        (
            "draft_email",
            email_input(json!(["not an address"])),
            "not an email",
        ),
        (
            "draft_sms",
            json!({ "to": standup, "body": "Hi", "device": "" }),
            "not a person",
        ),
        (
            "draft_event",
            json!({ "event": "", "title": "", "when": { "start": "", "end": "" }, "location": "", "attendees": [], "description": "", "calendar": "" }),
            "title",
        ),
    ] {
        api.one(&fake, None, tool, input);
        let r = last_result(&fake, true);
        assert!(r["error"].as_str().unwrap().contains(says), "{tool}: {r}");
    }
}

#[test]
fn only_an_agent_with_send_without_review_drafts_approved_items() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.agent("sender", SENDER);

    api.one(
        &fake,
        Some("clerk"),
        "draft_email",
        email_input(json!(["ann@example.com"])),
    );
    let r = last_result(&fake, false);
    assert_eq!(r["status"], "draft");
    assert_eq!(api.fields(r["id"].as_str().unwrap())["drafted_by"], "clerk");

    api.one(
        &fake,
        Some("sender"),
        "draft_email",
        email_input(json!(["ann@example.com"])),
    );
    let r = last_result(&fake, false);
    assert_eq!(r["status"], "ready", "{r}");
    assert!(r["message"].as_str().unwrap().contains("without review"));
    // The send handler takes it from ready; with no Google account it fails.
    let f = api.fields(r["id"].as_str().unwrap());
    assert_eq!(f["drafted_by"], "sender");
    assert!(
        ["ready", "sending", "failed"].contains(&f["status"].as_str().unwrap()),
        "{f}"
    );
}

#[test]
fn an_agent_cannot_make_its_own_draft_ready() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);

    // Clerk lets note changes go ahead without asking, so its edit is written at once, and the
    // writer refuses making it ready.
    api.one(
        &fake,
        Some("clerk"),
        "draft_email",
        email_input(json!(["ann@example.com"])),
    );
    let id = last_result(&fake, false)["id"].as_str().unwrap().to_owned();
    let note = api.note_json(&id);
    let mut doc = note["note"].clone();
    doc["fields"]["status"] = json!("ready");
    api.one(
        &fake,
        Some("clerk"),
        "propose_edit",
        json!({ "id": id, "base_version": note["version"], "note": doc.to_string() }),
    );
    let r = last_result(&fake, true);
    assert!(r.to_string().contains("status"), "{r}");
    assert_eq!(api.fields(&id)["status"], "draft");

    // Assistant, run by the user with note confirmation off, cannot make its draft ready either:
    // a change made without the user's Apply to an Outbox item is the agent's.
    api.send(
        "PUT",
        "/api/integrations/ai/settings",
        json!({ "confirm": { "notes": false } }),
    );
    api.one(
        &fake,
        None,
        "draft_email",
        email_input(json!(["ann@example.com"])),
    );
    let id = last_result(&fake, false)["id"].as_str().unwrap().to_owned();
    let note = api.note_json(&id);
    let mut doc = note["note"].clone();
    doc["fields"]["status"] = json!("ready");
    api.one(
        &fake,
        None,
        "propose_edit",
        json!({ "id": id, "base_version": note["version"], "note": doc.to_string() }),
    );
    last_result(&fake, true);
    assert_eq!(api.fields(&id)["status"], "draft");
}

#[test]
fn the_draft_tools_need_write_and_follow_the_tools_list() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("reader", READER);

    let read = api.token_of("viewer", "read");
    api.one_as(
        &read,
        &fake,
        None,
        "draft_email",
        email_input(json!(["ann@example.com"])),
    );
    let r = last_result(&fake, true);
    assert!(r["error"].as_str().unwrap().contains("write"), "{r}");
    let outbox = api.send("POST", "/api/query", json!({ "query": "from outbox" }));
    assert_eq!(outbox["total"], 0, "{outbox}");

    // An agent without them in its tools list is not offered them, and a call is not run.
    let n = fake.message_requests().len();
    api.one(
        &fake,
        Some("reader"),
        "draft_email",
        email_input(json!(["ann@example.com"])),
    );
    assert!(!tools_in(&fake, n).contains(&"draft_email".to_owned()));
    let r = last_result(&fake, true);
    assert!(r["error"].as_str().unwrap().contains("no tool"), "{r}");
}

#[test]
fn unattended_runs_are_offered_the_draft_tools() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // Every confirmation on: no change tool is offered, but drafting is not one.
    api.agent(
        "clerk",
        &CLERK
            .replace("    notes = false\n", "")
            .replace("    agents = false\n", ""),
    );
    let runs = api.get("/api/agents/clerk/runs");
    let tools: Vec<&str> = runs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for t in ["draft_email", "draft_sms", "draft_event"] {
        assert!(tools.contains(&t), "{t}: {tools:?}");
    }
    assert!(!tools.contains(&"propose_edit"), "{tools:?}");
}

#[test]
fn an_agent_cannot_give_another_the_bypass_it_lacks() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.agent("sender", SENDER);
    let helper = "label = \"Helper\"\n\npermissions {\n  tools = [\"draft_email\"]\n\n  outbox {\n    send_without_review = true\n  }\n}\n";

    api.one(
        &fake,
        Some("clerk"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": helper }),
    );
    let r = last_result(&fake, true);
    assert!(
        r["error"].as_str().unwrap().contains("send_without_review"),
        "{r}"
    );
    assert!(!api.agent_exists("helper"));

    // An agent that has the bypass may give it.
    api.one(
        &fake,
        Some("sender"),
        "propose_agent_change",
        json!({ "name": "helper", "wcl": helper }),
    );
    assert_eq!(last_result(&fake, false)["applied"], true);
    assert!(api.agent_exists("helper"));

    // Apply checks it again against the proposer.
    let forged = json!({
        "event": "agent_proposal",
        "id": "toolu_f",
        "kind": "agent",
        "op": "create",
        "name": "helper2",
        "exists": false,
        "before": null,
        "after": helper,
        "proposer": "clerk",
    });
    let res = api
        .client
        .post(api.server.url("/api/ai/apply"))
        .bearer_auth(api.server.admin_token())
        .json(&forged)
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(!api.agent_exists("helper2"));
}
