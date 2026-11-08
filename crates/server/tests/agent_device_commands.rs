//! S1 with the fake Anthropic provider: a run an agent starts cannot queue a device command.
//!
//! `vault::device_command` puts a command on a phone (a text message, a notification, a contact).
//! An agent's `run_script`, its `run_saved_script` (also a call to the shipped `outbox_send`
//! `send_sms` with a made-up item) and its `run_task` of a script that queues one are all runs
//! the agent started. The first two are refused and queue nothing. A task run by an agent keeps
//! its own author, so it still queues, as a scheduled run does (TV-97, O1).

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, PASSWORD, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

/// The refusal an agent's run gets from `vault::device_command`.
const REFUSED: &str = "vault::device_command is not available in a run an agent started";

/// An agent that may run its own code, which implies the saved scripts and tasks.
const SCRIPTER: &str =
    "label = \"Scripter\"\n\npermissions {\n  scripts {\n    run_code = true\n  }\n}\n";

/// An agent that may run saved scripts and tasks, but not its own code.
const SAVER: &str =
    "label = \"Saver\"\n\npermissions {\n  scripts {\n    run_saved = true\n  }\n}\n";

const ITEM: &str = r#"label = "Item"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// A device file that declares `send_sms` and `show_notification`, as the shipped Android file
/// does for those two commands.
const ANDROID: &str = r#"command "send_sms" {
  script = "device_messages"
}

command "show_notification" {
  script = "device_messages"
}
"#;

const DEVICE_MESSAGES: &str = "fn send_sms(payload: Value) -> Result[(), string] { Ok(()) }\n\
fn show_notification(payload: Value) -> Result[(), string] { Ok(()) }\n";

/// Queues one text when it runs: as a task, or when an agent runs it as a task.
const REMIND: &str = r#"use vault

fn run() -> Result[(), string] {
    let sms = Value::Map(#{ "to": Value::String("+61400000000"), "body": Value::String("Bins out") })
    vault::device_command("Pixel", "send_sms", sms)?
    Ok(())
}

fn queue_from_script() -> Result[(), string] {
    let sms = Value::Map(#{ "to": Value::String("+61400000001"), "body": Value::String("Hi") })
    vault::device_command("Pixel", "send_sms", sms)?
    Ok(())
}
"#;

/// The agent's own code, as `run_script` gets it: a text straight to the phone.
const DIRECT_SMS: &str = r#"use vault

fn main() -> Result[Value, string] {
    let sms = Value::Map(#{ "to": Value::String("+61400000002"), "body": Value::String("Direct") })
    vault::device_command("Pixel", "send_sms", sms)
}
"#;

fn seed(vault: &std::path::Path) {
    for (path, body) in [("types/item.wcl", ITEM), ("devices/android.wcl", ANDROID)] {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server = TestServer::start_with_env(seed, &[]);
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap();
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.secret(&name, &value);
        }
        api
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, self.server.url(path))
            .bearer_auth(self.token());
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn ok(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let (status, v) = self.send(method, path, body);
        assert!(status.is_success(), "{path}: {v}");
        v
    }

    fn put_text(&self, path: &str, text: &str) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.token())
            .header("content-type", "text/plain")
            .body(text.to_owned())
            .send()
            .unwrap();
        let status = res.status();
        let body = res.text().unwrap();
        assert!(status.is_success(), "{path}: {body}");
    }

    fn secret(&self, name: &str, value: &str) {
        self.ok(
            reqwest::Method::PUT,
            &format!("/api/secrets/{name}"),
            Some(json!({ "value": value })),
        );
    }

    fn agent(&self, name: &str, wcl: &str) {
        self.put_text(&format!("/api/agents/{name}"), wcl);
    }

    fn script(&self, name: &str, source: &str) {
        self.ok(
            reqwest::Method::PUT,
            &format!("/api/scripts/{name}"),
            Some(json!({ "source": source })),
        );
    }

    fn task(&self, name: &str, wcl: &str) {
        self.put_text(&format!("/api/tasks/{name}"), wcl);
    }

    /// Pair a phone and let it send texts, the way a user does in Settings > Devices.
    fn pair_with_sms(&self) -> String {
        let out = Client::new()
            .post(self.server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": "Pixel", "platform": "android" }))
            .send()
            .unwrap()
            .json::<Value>()
            .unwrap();
        let device = out["device_id"].as_str().unwrap().to_owned();
        self.ok(
            reqwest::Method::PATCH,
            &format!("/api/devices/{device}"),
            Some(json!({ "settings": { "commands": ["send_sms", "show_notification"] } })),
        );
        device
    }

    /// Every command the phone has, as the admin sees them.
    fn commands(&self, device: &str) -> Vec<Value> {
        let out = self.ok(
            reqwest::Method::GET,
            &format!("/api/devices/{device}/commands"),
            None,
        );
        out["commands"].as_array().unwrap().clone()
    }

    fn chat_as(&self, agent: Option<&str>) -> Vec<(String, Value)> {
        let mut body = json!({ "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Do it." }] }] });
        if let Some(a) = agent {
            body["agent"] = json!(a);
        }
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(self.token())
            .json(&body)
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    /// One round of tool calls by `agent` as the admin, then an answer.
    fn calls(&self, fake: &FakeAnthropic, agent: &str, calls: &[(&str, Value)]) {
        let mut m = AnthropicMessage::new();
        for (i, (tool, input)) in calls.iter().enumerate() {
            m = m.tool_use(&format!("toolu_{i}"), tool, input.clone());
        }
        fake.push(m);
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(Some(agent));
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

/// The tool results the model got back in the last request: the error flag and the content.
fn results(fake: &FakeAnthropic) -> Vec<(bool, String)> {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    r["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "tool_result")
        .map(|c| (c["is_error"] == true, c["content"].to_string()))
        .collect()
}

fn seeded_scripts(api: &Api) {
    api.script("device_messages", DEVICE_MESSAGES);
    api.script("remind", REMIND);
    api.script(
        "outbox_send",
        include_str!("../../writer/defaults/outbox_send.ws"),
    );
    api.task(
        "remind",
        "schedule = \"0 7 * * *\"\nscript   = \"remind\"\npaused   = true\n",
    );
}

#[test]
fn an_agents_run_script_and_saved_script_cannot_queue_a_device_command() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);
    api.agent("saver", SAVER);
    seeded_scripts(&api);
    let device = api.pair_with_sms();
    assert!(api.commands(&device).is_empty());

    // The agent's own code, with the device allowed to send texts.
    api.calls(
        &fake,
        "scripter",
        &[(
            "run_script",
            json!({ "code": DIRECT_SMS, "function": null, "args": null }),
        )],
    );
    let r = results(&fake);
    assert!(r[0].0, "{}", r[0].1);
    assert!(r[0].1.contains(REFUSED), "{}", r[0].1);
    assert!(api.commands(&device).is_empty());

    // A saved script's function that queues a command, run by an agent with only run_saved.
    api.calls(
        &fake,
        "saver",
        &[(
            "run_saved_script",
            json!({ "script": "remind", "function": "queue_from_script", "args": null }),
        )],
    );
    let r = results(&fake);
    assert!(r[0].0, "{}", r[0].1);
    assert!(r[0].1.contains(REFUSED), "{}", r[0].1);
    assert!(api.commands(&device).is_empty());

    // The shipped send_sms of outbox_send with a made-up item: no Outbox item, no review.
    let item = json!({
        "id": "01FAKEITEM",
        "note": {
            "fields": { "phone": "+61400000003", "device": "" },
            "blocks": [{
                "kind": "content",
                "labels": ["body"],
                "blocks": [{ "kind": "paragraph", "fields": { "text": "Sent by an agent" } }]
            }]
        }
    });
    api.calls(
        &fake,
        "scripter",
        &[(
            "run_saved_script",
            json!({
                "script": "outbox_send",
                "function": "send_sms",
                "args": json!([item]).to_string(),
            }),
        )],
    );
    // run_saved_script refuses the shipped handler script before it runs (TV-100), so the
    // device_command guard is not reached; nothing is queued either way.
    let r = results(&fake);
    assert!(r[0].0, "{}", r[0].1);
    assert_eq!(r[0].1["kind"], "not_allowed", "{}", r[0].1);
    assert!(
        r[0].1["error"]
            .as_str()
            .unwrap()
            .contains("built-in handler script"),
        "{}",
        r[0].1
    );
    assert!(api.commands(&device).is_empty());

    // The refusals are logged as failed runs, not as successes.
    let log = api.ok(
        reqwest::Method::GET,
        "/api/activity?kind=ai.script&limit=50",
        None,
    );
    let outcomes: Vec<&Value> = log["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["detail"]["outcome"])
        .collect();
    assert!(!outcomes.is_empty(), "{log}");
    assert!(outcomes.iter().all(|o| *o != "ok"), "{log}");
}

#[test]
fn a_task_an_agent_runs_still_queues_its_device_command() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);
    seeded_scripts(&api);
    let device = api.pair_with_sms();

    // The task keeps its own author when an agent runs it, so the guard does not apply.
    api.calls(
        &fake,
        "scripter",
        &[("run_task", json!({ "name": "remind" }))],
    );
    let r = results(&fake);
    assert!(!r[0].0, "{}", r[0].1);
    let queued = api.commands(&device);
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0]["name"], "send_sms");
    assert_eq!(queued[0]["source"], "task:remind");
    assert_eq!(queued[0]["payload"]["body"], "Bins out");
}
