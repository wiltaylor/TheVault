//! S1 with S7 and the fake Anthropic: agent triggers run on the server (agents G9 to G12). A note
//! event starts an unattended turn on a new thread with the event line; the turn is offered no
//! change tool whose confirmation switch is on; a change tool whose switch is off applies at once,
//! authored by the agent, and fires no further trigger; runs are recorded and listed; a failed
//! run leaves one notification linking to its thread.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, AnthropicReply, FakeAnthropic, TestAccount, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MEMO: &str = r#"label = "Memo"

field status {
  kind = "string"
}

template default {
  tab "Main" {
    slot "title"
    slot "status"
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

    /// Move the test clock on, which runs whatever is due and waits for it.
    fn advance(&self, by: &str) {
        self.send("POST", "/api/test/clock", json!({ "advance": by }));
    }

    fn runs(&self, agent: &str) -> Value {
        self.get(&format!("/api/agents/{agent}/runs"))
    }
}

fn tool_names(fake: &FakeAnthropic, request: usize) -> Vec<String> {
    fake.message_requests()[request].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The text of the first user message of request `request`.
fn first_user_text(fake: &FakeAnthropic, request: usize) -> String {
    let r = fake.message_requests()[request].json();
    let first = &r["messages"][0];
    assert_eq!(first["role"], "user");
    first["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

const CLERK: &str = r#"label = "Clerk"

permissions {
  confirm {
    notes = false
  }
}

trigger opened {
  on     = ["create", "update"]
  type   = "memo"
  where  = "status == \"open\""
  prompt = "Tidy this memo."
}
"#;

#[test]
fn a_note_event_runs_the_agent_unattended_on_a_new_thread() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("clerk", CLERK);
    let (id, version) = api.memo("Roof", "open");
    // A memo the condition leaves out starts nothing.
    api.memo("Gutters", "closed");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_e",
        "propose_edit",
        json!({
            "id": id, "base_version": version,
            "note": json!({ "fields": { "title": "Roof (tidied)", "status": "open" } }).to_string(),
        }),
    ));
    fake.push(AnthropicMessage::text("Tidied."));
    api.advance("PT10S");
    assert!(fake.message_requests().is_empty(), "the run waits 30 s");
    api.advance("PT25S");
    assert_eq!(fake.message_requests().len(), 2);
    assert_eq!(
        first_user_text(&fake, 0),
        format!("Tidy this memo.\nEvent: create of note {id} (type memo, title \"Roof\")")
    );

    // Offered the read tools and the note changes its switch lets through, nothing that would
    // wait for a user.
    let tools = tool_names(&fake, 0);
    for t in [
        "search_notes",
        "query_notes",
        "get_note",
        "propose_edit",
        "propose_create",
    ] {
        assert!(tools.contains(&t.to_owned()), "{t} in {tools:?}");
    }
    for t in [
        "propose_archive",
        "propose_delete",
        "propose_type_change",
        "propose_taxonomy_change",
        "propose_taxonomy_delete",
        "propose_agent_change",
        "propose_agent_delete",
        "download_asset",
    ] {
        assert!(!tools.contains(&t.to_owned()), "{t} not in {tools:?}");
    }

    // The edit was made at once, by the agent, and fired nothing more.
    let note = api.get(&format!("/api/notes/{id}"));
    assert_eq!(note["note"]["fields"]["title"], "Roof (tidied)", "{note}");
    let history = api.get(&format!("/api/notes/{id}/history"));
    let current = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["current"] == true)
        .unwrap()
        .clone();
    assert_eq!(current["author"]["kind"], "agent", "{current}");
    api.advance("PT31S");
    assert_eq!(fake.message_requests().len(), 2);

    // The thread holds the run; the run is recorded with it.
    let threads = api.get("/api/agents/clerk/threads");
    let threads = threads["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "{threads:?}");
    assert_eq!(threads[0]["origin"], "trigger:opened");
    assert_eq!(threads[0]["pending"], 0);
    let thread = threads[0]["id"].as_str().unwrap();
    let runs = api.runs("clerk");
    let opened = &runs["triggers"][0];
    assert_eq!(opened["name"], "opened");
    assert_eq!(opened["kind"], "event");
    let run = &opened["runs"][0];
    assert_eq!(run["outcome"], "succeeded", "{runs}");
    assert_eq!(run["thread"], thread);
    assert_eq!(runs["tools"].as_array().unwrap().len(), tools.len());
    let read = api.get(&format!("/api/agents/clerk/threads/{thread}"));
    let messages = read["messages"].as_array().unwrap();
    assert!(messages.len() >= 3, "{read}");
    // A run that went well sends no notification.
    let notes = api.get("/api/notifications");
    assert!(notes["items"].as_array().unwrap().is_empty(), "{notes}");
}

#[test]
fn a_change_tool_whose_switch_is_on_is_left_out_of_a_trigger_run() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "reader",
        "label = \"Reader\"\n\npermissions {\n  confirm {\n    delete = false\n  }\n}\n\ntrigger any {\n  on = [\"create\"]\n  prompt = \"Read it.\"\n}\n",
    );
    let tools = api.runs("reader")["tools"].clone();
    let tools: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(tools.contains(&"propose_delete"), "{tools:?}");
    assert!(tools.contains(&"get_note"), "{tools:?}");
    assert!(!tools.contains(&"propose_edit"), "{tools:?}");
    assert!(!tools.contains(&"propose_create"), "{tools:?}");
    fake.push(AnthropicMessage::text("Read."));
    api.memo("Roof", "open");
    api.advance("PT31S");
    let offered = tool_names(&fake, 0);
    assert_eq!(offered, tools, "the run is offered what the route says");
}

#[test]
fn a_failed_run_is_recorded_and_notified_once() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "owl",
        "label = \"Night owl\"\n\ntrigger nightly {\n  schedule = \"0 2 * * *\"\n  prompt = \"Summarise the day.\"\n}\n",
    );
    // The scheduler reads the new agent, then the night comes.
    api.advance("PT1S");
    fake.push(AnthropicReply::error(
        400,
        "invalid_request_error",
        "Your credit balance is too low.",
    ));
    api.advance("PT90M");
    assert_eq!(fake.message_requests().len(), 1);
    assert_eq!(first_user_text(&fake, 0), "Summarise the day.");
    let runs = api.runs("owl");
    let run = &runs["triggers"][0]["runs"][0];
    assert_eq!(run["outcome"], "failed", "{runs}");
    let thread = run["thread"].as_str().unwrap();
    let threads = api.get("/api/agents/owl/threads");
    assert_eq!(threads["threads"][0]["title"], "nightly 2026-01-05 02:00");
    let notes = api.get("/api/notifications");
    let notes = notes["items"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["title"], "Night owl\u{2019}s nightly run failed");
    assert_eq!(notes[0]["url"], format!("/agents/owl/chat?thread={thread}"));
}

#[test]
fn an_unattended_run_starts_sub_agents_offered_only_what_it_may_make() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent(
        "chief",
        "label = \"Chief\"\n\npermissions {\n  confirm {\n    delete = false\n  }\n\n  subagents {\n    allow = true\n  }\n}\n\ntrigger any {\n  on = [\"create\"]\n  prompt = \"Look into it.\"\n}\n",
    );
    let tools = api.runs("chief")["tools"].clone();
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "start_subagent"),
        "{tools}"
    );
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_0",
        "start_subagent",
        json!({ "task": "Check the memo", "instructions": null, "agent": null, "permissions": null, "model": null }),
    ));
    fake.push(AnthropicMessage::text("Checked."));
    fake.push(AnthropicMessage::text("Done."));
    api.memo("Roof", "open");
    api.advance("PT31S");
    assert_eq!(fake.message_requests().len(), 3);
    // The sub agent, like its run, has nobody to ask (subagents U8): it is offered the change
    // tools it makes itself, not those that would only propose, and never start_subagent.
    let sub = tool_names(&fake, 1);
    assert!(sub.contains(&"propose_delete".to_owned()), "{sub:?}");
    assert!(sub.contains(&"get_note".to_owned()), "{sub:?}");
    assert!(!sub.contains(&"propose_create".to_owned()), "{sub:?}");
    assert!(!sub.contains(&"start_subagent".to_owned()), "{sub:?}");
    let runs = api.runs("chief");
    assert_eq!(
        runs["triggers"][0]["runs"][0]["outcome"], "succeeded",
        "{runs}"
    );
}
