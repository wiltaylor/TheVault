//! S1 with the fake Anthropic and the test clock: a script an agent starts notifies under the
//! agent's limits (TV-76). Its notifications count toward the agent's hour and day limits, shared
//! with `notify_user`, and toward the day cap of all agents together (TV-75), and a refused call
//! stores nothing. A scheduled task, a task an agent runs with `run_task`, a handler and a user's
//! own tool run keep their sources and are not limited.

use std::ffi::OsStr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

/// Monday 2026-01-05, half past midnight UTC.
const START: &str = "2026-01-05T00:30:00Z";

const MODEL: &str = "claude-opus-5";

/// An agent that runs saved scripts, but not its own code.
const SAVER: &str =
    "label = \"Saver\"\n\npermissions {\n  scripts {\n    run_saved = true\n  }\n}\n";

/// An agent that runs its own code, which implies the saved scripts and tasks.
const SCRIPTER: &str =
    "label = \"Scripter\"\n\npermissions {\n  scripts {\n    run_code = true\n  }\n}\n";

/// An agent with no permissions of its own.
const THIRD: &str = "label = \"Third\"\n";

/// A bill notifies eleven times when it is created: a handler, which is not limited.
const BILL: &str = r#"label = "Bill"

handler "after_create" {
  script = "bill_burst"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// A user's tool, which runs the script `burst_tool`.
const BURST_TOOL: &str = r#"description = "Send eleven notices."
script = "burst_tool"
function = "run"
"#;

/// `send_many(n, title)` notifies `n` times, titled "<title> 0" to "<title> <n - 1>", and stops
/// at the first refusal.
const SEND_MANY: &str = r#"use vault

fn send_many(n: int, title: string) -> Result[(), string] {
    for i in 0..n {
        vault::notify(fmt("{} {}", title, i), "", "")?
    }
    Ok(())
}
"#;

/// The saved script `ping`: `ping(n)` sends `n` notifications.
fn ping_source() -> String {
    format!(
        "{SEND_MANY}\nfn ping(n: int) -> Result[(), string] {{\n    send_many(n, \"Ping\")\n}}\n"
    )
}

/// The agent's own code: it sends `n` notifications when it runs.
fn scripted(n: usize) -> String {
    format!(
        "{SEND_MANY}\nfn main() -> Result[Value, string] {{\n    send_many({n}, \"Scripted\")?\n    \
         Ok(Value::Null)\n}}\n"
    )
}

fn seed(vault: &Path) {
    let bill_burst = format!(
        "{SEND_MANY}\nfn after_create(note: Value) -> Result[(), string] {{\n    \
         send_many(11, \"Handler\")\n}}\n"
    );
    for (path, body) in [
        ("types/bill.wcl", BILL.to_owned()),
        ("scripts/bill_burst.ws", bill_burst),
    ] {
        let path = vault.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

/// `run_saved_script` for the saved script `ping`, with `n` as its argument.
fn ping(n: usize) -> Value {
    json!({ "script": "ping", "function": "ping", "args": format!("[{n}]") })
}

/// `run_script` with the agent's own code of `n` notifications.
fn run_code(n: usize) -> Value {
    json!({ "code": scripted(n), "function": null, "args": null })
}

fn hello(title: &str) -> Value {
    json!({ "title": title, "body": "", "note_id": "" })
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        let server =
            TestServer::start_with_args(seed, &[OsStr::new("--test-clock"), OsStr::new(START)]);
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap();
        let api = Api { server, client };
        for (name, value) in AiSeed::new()
            .account(TestAccount::anthropic(fake).model(MODEL))
            .secrets()
        {
            api.ok(
                reqwest::Method::PUT,
                &format!("/api/secrets/{name}"),
                Some(json!({ "value": value })),
            );
        }
        api
    }

    fn ok(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let mut req = self
            .client
            .request(method, self.server.url(path))
            .bearer_auth(self.server.admin_token());
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().unwrap();
        let status = res.status();
        let v: Value = res.json().unwrap_or(Value::Null);
        assert!(status.is_success(), "{path}: {v}");
        v
    }

    fn put_text(&self, path: &str, text: &str) {
        let res = self
            .client
            .put(self.server.url(path))
            .bearer_auth(self.server.admin_token())
            .header("content-type", "text/plain")
            .body(text.to_owned())
            .send()
            .unwrap();
        let status = res.status();
        let body = res.text().unwrap();
        assert!(status.is_success(), "{path}: {body}");
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

    /// Move the test clock on, which runs whatever is due.
    fn advance(&self, by: &str) {
        self.ok(
            reqwest::Method::POST,
            "/api/test/clock",
            Some(json!({ "advance": by })),
        );
    }

    /// The stored notifications, newest first.
    fn notifications(&self) -> Vec<Value> {
        let list = self.ok(reqwest::Method::GET, "/api/notifications?limit=100", None);
        list["items"].as_array().unwrap().clone()
    }

    /// How many stored notifications have `source`.
    fn count_from(&self, source: &str) -> usize {
        self.notifications()
            .iter()
            .filter(|n| n["source"] == source)
            .count()
    }

    /// One turn by `agent`, the admin's, whose first round makes `calls`, then answers. Returns
    /// each tool result of that round as its error flag and its content.
    fn calls(
        &self,
        fake: &FakeAnthropic,
        agent: &str,
        calls: &[(&str, Value)],
    ) -> Vec<(bool, String)> {
        let mut m = AnthropicMessage::new();
        for (i, (tool, input)) in calls.iter().enumerate() {
            m = m.tool_use(&format!("toolu_{i}"), tool, input.clone());
        }
        fake.push(m);
        fake.push(AnthropicMessage::text("Done."));
        let body = json!({
            "agent": agent,
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Do it." }] }],
        });
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(self.server.admin_token())
            .json(&body)
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // Read the whole stream, so the turn is over before its results are read.
        let _ = res.text().unwrap();
        results(fake)
    }
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

/// The result is an error that says `says`, and that nothing was sent.
fn assert_limited(result: &(bool, String), says: &str) {
    assert!(result.0, "{}", result.1);
    assert!(result.1.contains(says), "{} lacks {says:?}", result.1);
    assert!(result.1.contains("Nothing was sent"), "{}", result.1);
}

#[test]
fn a_scripts_notifications_count_against_the_agents_hour_and_day_limits() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("saver", SAVER);
    api.agent("scripter", SCRIPTER);
    api.agent("third", THIRD);
    api.script("ping", &ping_source());

    // In one hour, a saved script sends 11 and the eleventh is refused: 10 are stored.
    let r = api.calls(&fake, "saver", &[("run_saved_script", ping(11))]);
    assert_limited(&r[0], "last hour");
    assert_eq!(api.count_from("agent:saver"), 10);

    // The agent's own code has its own budget in the same hour.
    let r = api.calls(&fake, "scripter", &[("run_script", run_code(11))]);
    assert_limited(&r[0], "last hour");
    assert_eq!(api.count_from("agent:scripter"), 10);

    // notify_user shares the agent's budget: the saver's hour is full, so it is refused too.
    let r = api.calls(&fake, "saver", &[("notify_user", hello("Hi"))]);
    assert!(r[0].0, "{}", r[0].1);
    assert!(r[0].1.contains("last hour"), "{}", r[0].1);
    assert_eq!(api.notifications().len(), 20);

    // Two more hours, ten from each agent in each.
    for _ in 0..2 {
        api.advance("PT61M");
        let r = api.calls(&fake, "saver", &[("run_saved_script", ping(10))]);
        assert!(!r[0].0, "{}", r[0].1);
        let r = api.calls(&fake, "scripter", &[("run_script", run_code(10))]);
        assert!(!r[0].0, "{}", r[0].1);
    }
    assert_eq!(api.count_from("agent:saver"), 30);
    assert_eq!(api.count_from("agent:scripter"), 30);

    // The hour has passed, but each agent has its 30 for the day.
    api.advance("PT61M");
    let r = api.calls(&fake, "saver", &[("run_saved_script", ping(1))]);
    assert_limited(&r[0], "24 hours");
    assert!(r[0].1.contains("limit is 30 a day"), "{}", r[0].1);

    // All agents together have 60 for the day, so a third agent's call is refused too.
    let r = api.calls(&fake, "third", &[("notify_user", hello("Hi"))]);
    assert!(r[0].0, "{}", r[0].1);
    assert!(r[0].1.contains("all agents together"), "{}", r[0].1);
    assert!(r[0].1.contains("60 a day"), "{}", r[0].1);
    assert_eq!(api.notifications().len(), 60);

    // A day after the first, the window is clear and the third agent may notify again.
    api.advance("PT24H");
    let r = api.calls(&fake, "third", &[("notify_user", hello("Again"))]);
    assert!(!r[0].0, "{}", r[0].1);
    assert_eq!(api.count_from("agent:third"), 1);
}

#[test]
fn a_task_a_handler_and_a_user_tool_run_keep_their_sources_and_are_not_limited() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("scripter", SCRIPTER);
    let burst = format!(
        "{SEND_MANY}\nfn run() -> Result[(), string] {{\n    send_many(11, \"Scheduled\")\n}}\n"
    );
    api.script("burst", &burst);
    api.task(
        "burst",
        "label = \"Burst\"\nschedule = \"0 7 * * *\"\nscript = \"burst\"\nretries = 0\n",
    );

    // A scheduled run: eleven notifications from the task, and no limit.
    let run = api.ok(
        reqwest::Method::POST,
        "/api/tasks/burst/run?wait=true",
        None,
    );
    assert_eq!(run["outcome"], "succeeded", "{run}");
    assert_eq!(api.count_from("task:burst"), 11);

    // The same task run by an agent keeps the task's source, so it is not limited either.
    let r = api.calls(&fake, "scripter", &[("run_task", json!({ "name": "burst" }))]);
    assert!(!r[0].0, "{}", r[0].1);
    assert_eq!(api.count_from("task:burst"), 22);

    // A handler: a bill created in the vault notifies eleven times after the write.
    api.ok(
        reqwest::Method::POST,
        "/api/notes?type=bill",
        Some(json!({ "note": { "fields": { "title": "March" } } })),
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    while api.count_from("handler:bill.after_create") < 11 {
        assert!(Instant::now() < deadline, "no handler notice came");
        thread::sleep(Duration::from_millis(50));
    }

    // A user's own tool run, with the admin's token: eleven more, from no agent.
    let tool = format!(
        "{SEND_MANY}\nfn run(params: Value) -> Result[Value, string] {{\n    \
         send_many(11, \"Tool\")?\n    Ok(Value::Null)\n}}\n"
    );
    api.script("burst_tool", &tool);
    api.ok(
        reqwest::Method::PUT,
        "/api/tools/burst_tool",
        Some(json!({ "wcl": BURST_TOOL })),
    );
    let ran = api.ok(
        reqwest::Method::POST,
        "/api/tools/burst_tool/run",
        Some(json!({ "params": {} })),
    );
    assert_eq!(ran["ok"], true, "{ran}");

    let all = api.notifications();
    assert_eq!(all.len(), 44, "{all:?}");
    assert!(
        all.iter()
            .all(|n| !n["source"].as_str().unwrap().starts_with("agent:")),
        "{all:?}"
    );
}
