//! S1 with S7 and the fake Anthropic: calling an agent from a note (agents G16, G19). A saved
//! note with a committed call, a link `[@<Label> <request>](agent:<name>?call=<id>)`, starts one
//! run whose first message is the request, and the note is left as it was; a call id is answered
//! once, whatever is saved after; plain `@name` text, inline code and an agent's own writes call
//! no one; a type the trigger leaves out records the call with no run; `GET
//! /api/agents/calls/{call}` answers what became of a call, or 404.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

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

const LOG: &str = r#"label = "Log"

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

const MODEL: &str = "claude-opus-5";

const CLERK: &str = r#"label = "Clerk"

permissions {
  confirm {
    notes = false
  }
}

trigger asked {
  mention = true
  type    = "memo"
  prompt  = "Keep it short."
}
"#;

const OWL: &str = r#"label = "Night owl"

trigger asked {
  mention = true
}
"#;

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
                std::fs::write(vault.join("types/log.wcl"), LOG).unwrap();
            },
            &[OsStr::new("--test-clock"), OsStr::new(START)],
        );
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

    fn status(&self, path: &str) -> u16 {
        self.request("GET", path, None).status().as_u16()
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Response {
        let url = self.server.url(path);
        let req = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
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

    /// A note of `type_name` whose body is one paragraph per text.
    fn note(&self, type_name: &str, title: &str, paragraphs: &[&str]) -> String {
        let v = self.send(
            "POST",
            &format!("/api/notes?type={type_name}"),
            json!({ "note": note_json(title, paragraphs) }),
        );
        v["id"].as_str().unwrap().to_owned()
    }

    fn save(&self, id: &str, title: &str, paragraphs: &[&str]) {
        let version = self.get(&format!("/api/notes/{id}"))["version"].clone();
        self.send(
            "PUT",
            &format!("/api/notes/{id}"),
            json!({ "note": note_json(title, paragraphs), "base_version": version }),
        );
    }

    /// The text of each paragraph of note `id`'s body.
    fn paragraphs(&self, id: &str) -> Vec<String> {
        let note = self.get(&format!("/api/notes/{id}"));
        note["note"]["blocks"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["fields"]["text"].as_str().unwrap().to_owned())
            .collect()
    }

    fn threads(&self, agent: &str) -> Vec<Value> {
        self.get(&format!("/api/agents/{agent}/threads"))["threads"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// Move the test clock on, which runs whatever is due and waits for it.
    fn advance(&self, by: &str) {
        self.send("POST", "/api/test/clock", json!({ "advance": by }));
    }
}

fn note_json(title: &str, paragraphs: &[&str]) -> Value {
    let blocks: Vec<Value> = paragraphs
        .iter()
        .map(|p| json!({ "kind": "paragraph", "fields": { "text": p } }))
        .collect();
    json!({
        "fields": { "title": title },
        "blocks": [{ "kind": "content", "labels": ["body"], "blocks": blocks }],
    })
}

/// The text of the first user message of each request sent to the model.
fn first_messages(fake: &FakeAnthropic) -> Vec<String> {
    fake.message_requests()
        .iter()
        .map(|r| {
            r.json()["messages"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

const CALL: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const CALL2: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";

/// A call of `agent` with link text `text` and call id `call`, as the editor commits it.
fn call(text: &str, agent: &str, call: &str) -> String {
    format!("[{text}](agent:{agent}?call={call})")
}

#[test]
fn a_committed_call_starts_one_run_and_leaves_the_note_as_it_was() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    assert_eq!(api.status(&format!("/api/agents/calls/{CALL}")), 404);
    let asked = call("@Clerk please list what to buy.", "clerk", CALL);
    let id = api.note("memo", "Roof", &["Gutters are full.", &asked]);
    let version = api.get(&format!("/api/notes/{id}"))["version"].clone();
    fake.push(AnthropicMessage::text("Ladder, gloves."));
    api.advance("PT1S");

    // One thread, from the call, holding the run.
    let threads = api.threads("clerk");
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(threads[0]["origin"], "mention");
    assert_eq!(threads[0]["title"], "Roof: please list what to buy.");
    let thread = threads[0]["id"].as_str().unwrap();
    assert_eq!(
        first_messages(&fake),
        [format!(
            "Keep it short.\nplease list what to buy.\nCalled from note {id} (type memo, title \"Roof\")."
        )]
    );
    let read = api.get(&format!("/api/agents/clerk/threads/{thread}"));
    assert!(
        read["messages"].to_string().contains("Ladder, gloves."),
        "{read}"
    );
    let runs = api.get("/api/agents/clerk/runs");
    let trigger = &runs["triggers"][0];
    assert_eq!(trigger["kind"], "mention");
    assert_eq!(trigger["runs"][0]["thread"], thread);
    assert_eq!(trigger["runs"][0]["outcome"], "succeeded", "{runs}");

    // The note is as the user saved it.
    let note = api.get(&format!("/api/notes/{id}"));
    assert_eq!(note["version"], version);
    assert_eq!(
        api.paragraphs(&id),
        ["Gutters are full.".to_owned(), asked.clone()]
    );
    assert_eq!(
        api.get(&format!("/api/agents/calls/{CALL}")),
        json!({ "agent": "clerk", "note": id, "thread": thread, "running": false })
    );

    // Saving again, editing the call's text, or the call in another note calls no one.
    api.save(&id, "Roof", &["Gutters are full.", &asked, "Bought them."]);
    api.advance("PT1S");
    let edited = call("@Clerk list what to buy, and where.", "clerk", CALL);
    api.save(&id, "Roof", &[&edited]);
    api.advance("PT1S");
    api.note("memo", "Copy", &[&edited]);
    api.advance("PT1S");
    assert_eq!(fake.message_requests().len(), 1);
    assert_eq!(api.threads("clerk").len(), 1);
}

#[test]
fn plain_text_code_and_other_links_call_no_one() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.note(
        "memo",
        "Mail",
        &[
            "@assistant and @clerk tidy this.",
            &format!("Type `{}` to call it.", call("@Clerk x", "clerk", CALL)),
            "See [@Clerk](/agents/clerk/chat?thread=01JB8Z4K2MQ7V3X9T5R6W8Y2ZD).",
        ],
    );
    api.advance("PT10S");
    assert!(fake.message_requests().is_empty());
    assert!(api.threads("clerk").is_empty());
    assert!(api.threads("assistant").is_empty());
    assert_eq!(api.status(&format!("/api/agents/calls/{CALL}")), 404);
}

#[test]
fn a_call_written_by_the_agent_itself_calls_no_one() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.note(
        "memo",
        "Roof",
        &[&call("@Clerk start a memo for the gutters.", "clerk", CALL)],
    );
    // The run writes a memo that calls the agent again, as the agent: that calls no one (G11).
    let again = call("@Clerk look at this later.", "clerk", CALL2);
    let memo = note_json("Gutters", &[&again]);
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_c",
        "propose_create",
        json!({ "type": "memo", "note": memo.to_string() }),
    ));
    fake.push(AnthropicMessage::text("Started it."));
    api.advance("PT1S");
    assert_eq!(fake.message_requests().len(), 2);
    let found = api.get("/api/notes?type=memo");
    let rows = found["notes"].as_array().unwrap().clone();
    assert!(rows.iter().any(|r| r["title"] == "Gutters"), "{found}");
    api.advance("PT10S");
    assert_eq!(
        fake.message_requests().len(),
        2,
        "the agent's own call starts nothing"
    );
    assert_eq!(api.threads("clerk").len(), 1);
    assert_eq!(api.status(&format!("/api/agents/calls/{CALL2}")), 404);
}

#[test]
fn two_calls_in_one_note_start_two_runs() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.agent("owl", OWL);
    let id = api.note(
        "memo",
        "Plan",
        &[
            &format!("{} and the rest.", call("@Clerk split this", "clerk", CALL)),
            &call("@Night owl check the dates", "owl", CALL2),
        ],
    );
    api.advance("PT1S");
    let (clerk, owl) = (api.threads("clerk"), api.threads("owl"));
    assert_eq!((clerk.len(), owl.len()), (1, 1));
    let mut firsts = first_messages(&fake);
    firsts.sort();
    assert_eq!(
        firsts,
        [
            format!(
                "Keep it short.\nsplit this\nContext: @Clerk split this and the rest.\nCalled from note {id} (type memo, title \"Plan\")."
            ),
            format!("check the dates\nCalled from note {id} (type memo, title \"Plan\")."),
        ]
    );
}

#[test]
fn a_type_the_trigger_leaves_out_records_the_call_with_no_run() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    let asked = call("@Clerk tidy this.", "clerk", CALL);
    let id = api.note("log", "Day", &[&asked]);
    api.advance("PT10S");
    assert!(fake.message_requests().is_empty());
    assert_eq!(api.paragraphs(&id), [asked]);
    assert_eq!(
        api.get(&format!("/api/agents/calls/{CALL}")),
        json!({ "agent": "clerk", "note": id, "thread": null, "running": false })
    );
    // The agents list says which types may call each agent, for the editor's picker.
    let agents = api.get("/api/agents");
    let clerk = agents["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "clerk")
        .unwrap()
        .clone();
    assert_eq!(clerk["mention"], json!({ "types": ["memo"] }));
    let assistant = &agents["agents"][0];
    assert_eq!(
        assistant["mention"],
        json!({ "types": null }),
        "every enabled agent can be called"
    );
}

#[test]
fn an_agent_with_no_mention_trigger_is_called_too() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    // No trigger at all: the agent answers a call from any type through the implicit trigger.
    api.agent("helper", "label = \"Helper\"\n");
    let id = api.note(
        "memo",
        "Shed",
        &[&call("@Helper what paint lasts outdoors?", "helper", CALL)],
    );
    fake.push(AnthropicMessage::text("Oil-based enamel."));
    api.advance("PT1S");

    let threads = api.threads("helper");
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(
        first_messages(&fake),
        [format!(
            "what paint lasts outdoors?\nCalled from note {id} (type memo, title \"Shed\")."
        )]
    );
}

#[test]
fn a_call_runs_on_the_agents_own_model_whatever_the_chat_box_chose() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "helper",
        "label = \"Helper\"\nprovider = \"anthropic\"\nmodel = \"claude-sonnet-5\"\n",
    );
    // A chat turn of the agent's on another model, picked in the chat box.
    fake.push(AnthropicMessage::text("Hi."));
    let res = api.request(
        "POST",
        "/api/ai/chat",
        Some(json!({
            "agent": "helper",
            "provider": "anthropic",
            "model": "claude-haiku-5",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
        })),
    );
    assert!(res.status().is_success());
    let events = res.text().unwrap();
    assert!(events.contains("event: done"), "{events}");
    // A call from a note runs on the agent's own model.
    api.note(
        "memo",
        "Shed",
        &[&call("@Helper what paint lasts outdoors?", "helper", CALL)],
    );
    fake.push(AnthropicMessage::text("Oil-based enamel."));
    api.advance("PT1S");
    let models: Vec<Value> = fake
        .message_requests()
        .iter()
        .map(|r| r.json()["model"].clone())
        .collect();
    assert_eq!(models, [json!("claude-haiku-5"), json!("claude-sonnet-5")]);
}
