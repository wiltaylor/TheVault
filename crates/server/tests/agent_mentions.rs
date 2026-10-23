//! S1 with S7 and the fake Anthropic: calling an agent from a note (agents G16). A saved note
//! with `@name` in its text gets the call replaced by a link to a new thread, whose run starts
//! with the instruction; a call is answered once; email addresses, inline code and an agent's
//! own writes call no one; two agents in one note start two runs; a type the trigger leaves out
//! calls no one.

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

#[test]
fn a_call_is_replaced_by_a_link_to_a_new_thread_and_answered_once() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    let id = api.note(
        "memo",
        "Roof",
        &["Gutters are full.", "@Clerk please list what to buy."],
    );
    fake.push(AnthropicMessage::text("Ladder, gloves."));
    api.advance("PT3S");
    assert!(
        fake.message_requests().is_empty(),
        "the note is idle 5 s first"
    );
    api.advance("PT3S");

    // One thread, from the call, holding the run.
    let threads = api.threads("clerk");
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(threads[0]["origin"], "mention");
    assert_eq!(threads[0]["title"], "Roof: you please list what to buy.");
    let thread = threads[0]["id"].as_str().unwrap();
    assert_eq!(
        api.paragraphs(&id),
        [
            "Gutters are full.".to_owned(),
            format!("[@Clerk](/agents/clerk/chat?thread={thread}) please list what to buy."),
        ]
    );
    assert_eq!(
        first_messages(&fake),
        [format!(
            "Keep it short.\nyou please list what to buy.\nCalled from note {id} (type memo, title \"Roof\"); the call was replaced with a link to this thread."
        )]
    );
    let read = api.get(&format!("/api/agents/clerk/threads/{thread}"));
    assert!(
        read["messages"].to_string().contains("Ladder, gloves."),
        "{read}"
    );
    let runs = api.get("/api/agents/clerk/runs");
    let asked = &runs["triggers"][0];
    assert_eq!(asked["kind"], "mention");
    assert_eq!(asked["runs"][0]["thread"], thread);
    assert_eq!(asked["runs"][0]["outcome"], "succeeded", "{runs}");

    // The rewrite is the agent's.
    let history = api.get(&format!("/api/notes/{id}/history"));
    let current = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["current"] == true)
        .unwrap()
        .clone();
    assert_eq!(current["author"]["kind"], "agent", "{current}");

    // Saving again calls no one: the call is a link now.
    let mut text = api.paragraphs(&id);
    text.push("Bought them.".to_owned());
    let text: Vec<&str> = text.iter().map(String::as_str).collect();
    api.save(&id, "Roof", &text);
    api.advance("PT10S");
    assert_eq!(fake.message_requests().len(), 1);
    assert_eq!(api.threads("clerk").len(), 1);
}

#[test]
fn addresses_code_and_links_call_no_one() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    let id = api.note(
        "memo",
        "Mail",
        &[
            "Write to bob@clerk.com today.",
            "Type `@clerk` to call it.",
            "See [@clerk](https://example.com/@clerk).",
            "a@clerk and @clerks and \\@clerk",
        ],
    );
    api.advance("PT10S");
    assert!(fake.message_requests().is_empty());
    assert!(api.threads("clerk").is_empty());
    assert!(api.paragraphs(&id)[0].contains("bob@clerk.com"));
}

#[test]
fn a_call_written_by_the_agent_itself_calls_no_one() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.note("memo", "Roof", &["@clerk start a memo for the gutters."]);
    // The run writes a memo that calls the agent again, as the agent: that calls no one (G11).
    let memo = note_json("Gutters", &["@clerk look at this later."]);
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_c",
        "propose_create",
        json!({ "type": "memo", "note": memo.to_string() }),
    ));
    fake.push(AnthropicMessage::text("Started it."));
    api.advance("PT6S");
    assert_eq!(fake.message_requests().len(), 2);
    let found = api.get("/api/notes?type=memo");
    let rows = found["notes"].as_array().unwrap().clone();
    let gutters = rows
        .iter()
        .find(|r| r["title"] == "Gutters")
        .unwrap_or_else(|| panic!("{found}"));
    let gutters = gutters["id"].as_str().unwrap();
    api.advance("PT10S");
    assert_eq!(
        fake.message_requests().len(),
        2,
        "the agent's own call starts nothing"
    );
    assert_eq!(
        api.paragraphs(gutters),
        ["@clerk look at this later.".to_owned()]
    );
    assert_eq!(api.threads("clerk").len(), 1);
}

#[test]
fn two_agents_called_in_one_note_start_two_runs() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    api.agent("owl", OWL);
    let id = api.note(
        "memo",
        "Plan",
        &[
            "@clerk and @owl, split this.",
            "(@owl) also check the dates",
        ],
    );
    api.advance("PT6S");
    let clerk = api.threads("clerk");
    let owl = api.threads("owl");
    assert_eq!((clerk.len(), owl.len()), (1, 1));
    let (c, o) = (
        clerk[0]["id"].as_str().unwrap(),
        owl[0]["id"].as_str().unwrap(),
    );
    // The owl's label is two words, so only its name calls it; both its calls make one run.
    assert_eq!(
        api.paragraphs(&id),
        [
            format!(
                "[@Clerk](/agents/clerk/chat?thread={c}) and [@Night owl](/agents/owl/chat?thread={o}), split this."
            ),
            format!("([@Night owl](/agents/owl/chat?thread={o})) also check the dates"),
        ]
    );
    let mut firsts = first_messages(&fake);
    firsts.sort();
    assert_eq!(firsts.len(), 2, "{firsts:?}");
    assert!(
        firsts[0].starts_with(
            "@clerk and you, split this.\n\n(you) also check the dates\nCalled from note"
        ),
        "{firsts:?}"
    );
    assert!(
        firsts[1].starts_with("Keep it short.\nyou and @owl, split this.\nCalled from note"),
        "{firsts:?}"
    );
}

#[test]
fn a_type_the_trigger_leaves_out_calls_no_one() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    let id = api.note("log", "Day", &["@clerk tidy this."]);
    api.advance("PT10S");
    assert!(fake.message_requests().is_empty());
    assert_eq!(api.paragraphs(&id), ["@clerk tidy this.".to_owned()]);
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
    let id = api.note("memo", "Shed", &["@helper what paint lasts outdoors?"]);
    fake.push(AnthropicMessage::text("Oil-based enamel."));
    api.advance("PT6S");

    let threads = api.threads("helper");
    assert_eq!(threads.len(), 1, "{threads:?}");
    let thread = threads[0]["id"].as_str().unwrap();
    assert_eq!(
        api.paragraphs(&id),
        [format!(
            "[@Helper](/agents/helper/chat?thread={thread}) what paint lasts outdoors?"
        )]
    );
    assert_eq!(
        first_messages(&fake),
        [format!(
            "you what paint lasts outdoors?\nCalled from note {id} (type memo, title \"Shed\"); the call was replaced with a link to this thread."
        )]
    );
}
