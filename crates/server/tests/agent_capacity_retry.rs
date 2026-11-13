//! S1 with S7 and the fake providers: an unattended agent run retries a model call that fails
//! with a transient error, after 60, 120 and 240 seconds (agents G12, `api::ai_retry`). The
//! server runs with `--test-clock`, under which the waits are over at once and are read from the
//! run's log. A used-up quota is not retried; the fourth failure fails the run as before; only the
//! failed call is made again, so a tool call of an earlier step never runs twice.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::{Value, json};
use testkit::{
    AiSeed, AnthropicMessage, AnthropicReply, FakeAnthropic, FakeOpenAi, OpenAiReply, TestAccount,
    TestServer,
};

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

/// An agent that runs nightly at two and may create notes without asking.
const OWL: &str = r#"label = "Night owl"

permissions {
  confirm {
    notes = false
  }
}

trigger nightly {
  schedule = "0 2 * * *"
  prompt   = "Summarise the day."
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(account: TestAccount) -> Api {
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
        for (name, value) in AiSeed::new().account(account).secrets() {
            api.server.seed_secret(&name, &value);
        }
        let res = api
            .client
            .put(api.server.url("/api/agents/owl"))
            .bearer_auth(api.server.admin_token())
            .header("content-type", "text/plain")
            .body(OWL)
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
        // The scheduler reads the new agent.
        api.advance("PT1S");
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
        assert!(
            res.status().is_success(),
            "GET {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap()
    }

    fn advance(&self, by: &str) {
        self.send("POST", "/api/test/clock", json!({ "advance": by }));
    }

    /// Let the night come: the nightly run starts and ends.
    fn night(&self) {
        self.advance("PT90M");
    }

    /// The nightly trigger's last run.
    fn run(&self) -> Value {
        let runs = self.get("/api/agents/owl/runs");
        runs["triggers"][0]["runs"][0].clone()
    }

    fn notifications(&self) -> Vec<Value> {
        self.get("/api/notifications")["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// The `agent.run` activity entries, newest first.
    fn run_entries(&self) -> Vec<Value> {
        self.get("/api/activity?limit=200&category=automation")["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "agent.run")
            .cloned()
            .collect()
    }
}

fn anthropic(fake: &FakeAnthropic) -> TestAccount {
    TestAccount::anthropic(fake).model("claude-opus-5")
}

/// A 529 `overloaded_error`. Its long `retry-after` keeps the provider client from retrying it
/// itself, so each is one attempt of the run's.
fn overloaded() -> AnthropicReply {
    AnthropicReply::error(529, "overloaded_error", "Overloaded").header("retry-after", "3600")
}

/// The wait lines of a run's log.
fn waits(run: &Value) -> Vec<String> {
    run["log"]
        .as_str()
        .unwrap()
        .lines()
        .filter(|l| l.contains("model at capacity"))
        .map(str::to_owned)
        .collect()
}

/// The messages request `i` sent.
fn sent_messages(fake: &FakeAnthropic, i: usize) -> Value {
    fake.message_requests()[i].json()["messages"].clone()
}

#[test]
fn a_run_at_capacity_once_waits_a_minute_and_succeeds() {
    let fake = FakeAnthropic::start();
    let api = Api::start(anthropic(&fake));
    fake.push(overloaded());
    fake.push(AnthropicMessage::text("All quiet."));
    api.night();
    assert_eq!(fake.message_requests().len(), 2);
    assert_eq!(sent_messages(&fake, 0), sent_messages(&fake, 1));
    let run = api.run();
    assert_eq!(run["outcome"], "succeeded", "{run}");
    let waits = waits(&run);
    assert_eq!(waits.len(), 1, "{run}");
    assert!(
        waits[0].starts_with("model at capacity; retrying in 60s (attempt 2 of 4): "),
        "{waits:?}"
    );
    assert!(waits[0].contains("Overloaded"), "{waits:?}");
    assert!(api.notifications().is_empty());
}

#[test]
fn three_failures_wait_one_two_and_four_minutes_then_succeed() {
    let fake = FakeAnthropic::start();
    let api = Api::start(anthropic(&fake));
    fake.push(overloaded());
    // A rate limit, and a provider error whose message says the model is at capacity.
    fake.push(AnthropicReply::rate_limited(3600));
    fake.push(
        AnthropicReply::error(500, "api_error", "The model is at capacity")
            .header("retry-after", "3600"),
    );
    fake.push(AnthropicMessage::text("All quiet."));
    api.night();
    assert_eq!(fake.message_requests().len(), 4);
    let run = api.run();
    assert_eq!(run["outcome"], "succeeded", "{run}");
    let waits = waits(&run);
    let heads: Vec<&str> = waits
        .iter()
        .map(|w| w.split(": ").next().unwrap())
        .collect();
    assert_eq!(
        heads,
        [
            "model at capacity; retrying in 60s (attempt 2 of 4)",
            "model at capacity; retrying in 120s (attempt 3 of 4)",
            "model at capacity; retrying in 240s (attempt 4 of 4)",
        ],
        "{run}"
    );
    assert!(api.notifications().is_empty());
}

#[test]
fn a_fourth_failure_fails_the_run_as_before() {
    let fake = FakeAnthropic::start();
    let api = Api::start(anthropic(&fake));
    for _ in 0..3 {
        fake.push(overloaded());
    }
    fake.push(
        AnthropicReply::error(529, "overloaded_error", "Still overloaded")
            .header("retry-after", "3600"),
    );
    api.night();
    assert_eq!(fake.message_requests().len(), 4);
    let run = api.run();
    assert_eq!(run["outcome"], "failed", "{run}");
    assert_eq!(run["message"], "Anthropic claude-opus-5: Still overloaded");
    assert_eq!(waits(&run).len(), 3, "{run}");
    let thread = run["thread"].as_str().unwrap();
    let notes = api.notifications();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["title"], "Night owl\u{2019}s nightly run failed");
    assert_eq!(
        notes[0]["body"],
        "Anthropic claude-opus-5: Still overloaded"
    );
    assert_eq!(notes[0]["url"], format!("/agents/owl/chat?thread={thread}"));
    let entries = api.run_entries();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["detail"]["outcome"], "failed");
    assert_eq!(
        entries[0]["detail"]["error"],
        "Anthropic claude-opus-5: Still overloaded"
    );
    assert_eq!(
        entries[0]["summary"],
        "Night owl\u{2019}s nightly run failed: Anthropic claude-opus-5: Still overloaded"
    );
}

#[test]
fn a_used_up_quota_is_not_retried() {
    let fake = FakeOpenAi::start();
    let api = Api::start(TestAccount::openai(&fake).model("gpt-5"));
    fake.push(OpenAiReply::error(
        429,
        "insufficient_quota",
        "You exceeded your current quota",
    ));
    fake.push(OpenAiReply::error(
        429,
        "insufficient_quota",
        "You exceeded your current quota",
    ));
    api.night();
    assert_eq!(fake.response_requests().len(), 1);
    let run = api.run();
    assert_eq!(run["outcome"], "failed", "{run}");
    assert!(waits(&run).is_empty(), "{run}");
    assert_eq!(api.notifications().len(), 1);
}

#[test]
fn a_rate_limit_from_openai_is_retried() {
    let fake = FakeOpenAi::start();
    let api = Api::start(TestAccount::openai(&fake).model("gpt-5"));
    fake.push(OpenAiReply::rate_limited(3600));
    fake.push(testkit::OpenAiResponse::text("All quiet."));
    api.night();
    assert_eq!(fake.response_requests().len(), 2);
    let run = api.run();
    assert_eq!(run["outcome"], "succeeded", "{run}");
    assert_eq!(waits(&run).len(), 1, "{run}");
}

#[test]
fn an_overload_mid_stream_before_any_tool_runs_is_retried_and_its_text_dropped() {
    let fake = FakeAnthropic::start();
    let api = Api::start(anthropic(&fake));
    // The stream breaks off with an overload part way through a tool call it asked for: the
    // call never ran, so the step is made again.
    fake.push(
        AnthropicMessage::text("Half an answer")
            .tool_use(
                "toolu_lost",
                "propose_create",
                json!({ "type": "memo", "note": json!({ "fields": { "title": "Lost" } }).to_string() }),
            )
            .error_after(7, "overloaded_error", "Overloaded"),
    );
    fake.push(AnthropicMessage::text("All quiet."));
    api.night();
    assert_eq!(fake.message_requests().len(), 2);
    assert_eq!(sent_messages(&fake, 0), sent_messages(&fake, 1));
    let run = api.run();
    assert_eq!(run["outcome"], "succeeded", "{run}");
    assert_eq!(waits(&run).len(), 1, "{run}");
    let memos = api.get("/api/notes?type=memo");
    assert!(memos["notes"].as_array().unwrap().is_empty(), "{memos}");
    let thread = run["thread"].as_str().unwrap();
    let read = api.get(&format!("/api/agents/owl/threads/{thread}"));
    let text = read["messages"].to_string();
    assert!(!text.contains("Half an answer"), "{read}");
    assert!(text.contains("All quiet."), "{read}");
}

#[test]
fn an_earlier_steps_tool_call_is_not_repeated_when_a_later_step_is_retried() {
    let fake = FakeAnthropic::start();
    let api = Api::start(anthropic(&fake));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_make",
        "propose_create",
        json!({ "type": "memo", "note": json!({ "fields": { "title": "Made once" } }).to_string() }),
    ));
    fake.push(overloaded());
    fake.push(overloaded());
    fake.push(AnthropicMessage::text("Made it."));
    api.night();
    assert_eq!(fake.message_requests().len(), 4);
    // The retried calls send the step before them as it was, its tool answered once.
    let second = sent_messages(&fake, 1);
    assert_eq!(second, sent_messages(&fake, 2));
    assert_eq!(second, sent_messages(&fake, 3));
    assert_eq!(
        second.to_string().matches("toolu_make").count(),
        2,
        "{second}"
    );
    let run = api.run();
    assert_eq!(run["outcome"], "succeeded", "{run}");
    assert_eq!(waits(&run).len(), 2, "{run}");
    let memos = api.get("/api/notes?type=memo");
    let memos = memos["notes"].as_array().unwrap();
    assert_eq!(memos.len(), 1, "{memos:?}");
    assert_eq!(memos[0]["title"], "Made once");
}

#[test]
fn interactive_chat_is_not_retried() {
    let fake = FakeAnthropic::start();
    let api = Api::start(anthropic(&fake));
    fake.push(overloaded());
    fake.push(AnthropicMessage::text("Too late."));
    let res = api
        .client
        .post(api.server.url("/api/ai/chat"))
        .bearer_auth(api.server.admin_token())
        .json(&json!({
            "agent": "owl",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Hi" }] }],
        }))
        .send()
        .unwrap();
    assert!(res.status().is_success());
    let body = res.text().unwrap();
    assert!(body.contains("event: error"), "{body}");
    assert_eq!(fake.message_requests().len(), 1);
}
