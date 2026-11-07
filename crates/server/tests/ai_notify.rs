//! S1 with S7 and the fake Anthropic: the chat's `notify_user` tool (TV-32 S1). It stores a
//! notification under the bell at once, from `agent:<name>`, opening a note the agent may read;
//! it refuses a bad title or body, a missing note or one of a type the agent may not read, and a
//! caller without `write`, sending nothing; it sends at most 3 per turn, 10 per hour and 30 per
//! day per agent, and a refused call says which limit; it follows the tools list; and an
//! unattended trigger run is offered it and uses it.

use std::ffi::OsStr;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

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

/// An agent that reads only memos.
const MEMOS_ONLY: &str = r#"label = "Memos only"

permissions {
  types = ["memo"]
}
"#;

/// An agent whose tools list leaves notify_user out.
const READER: &str = r#"label = "Reader"

permissions {
  tools = ["get_note"]
}
"#;

/// An agent run unattended when a memo opens, with every confirmation on.
const WATCHER: &str = r#"label = "Watcher"

trigger opened {
  on     = ["create"]
  type   = "memo"
  where  = "status == \"open\""
  prompt = "Tell the user about this memo."
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

    fn note(&self, type_name: &str, fields: Value) -> String {
        let v = self.send(
            "POST",
            &format!("/api/notes?type={type_name}"),
            json!({ "note": { "fields": fields } }),
        );
        v["id"].as_str().unwrap().to_owned()
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

    /// Move the test clock on, which runs whatever is due and waits for it.
    fn advance(&self, by: &str) {
        self.send("POST", "/api/test/clock", json!({ "advance": by }));
    }

    /// The stored notifications, newest first.
    fn notifications(&self) -> Vec<Value> {
        self.get("/api/notifications?limit=100")["items"]
            .as_array()
            .unwrap()
            .clone()
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

    /// One turn by `agent` as `token` whose first round calls notify_user once for each input,
    /// then answers: the turn's events.
    fn notify_as(
        &self,
        token: &str,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        inputs: &[Value],
    ) -> Vec<(String, Value)> {
        let mut m = AnthropicMessage::new();
        for (i, input) in inputs.iter().enumerate() {
            m = m.tool_use(&format!("toolu_{i}"), "notify_user", input.clone());
        }
        fake.push(m);
        fake.push(AnthropicMessage::text("Done."));
        self.chat_as(token, agent)
    }

    fn notify(
        &self,
        fake: &FakeAnthropic,
        agent: Option<&str>,
        inputs: &[Value],
    ) -> Vec<(String, Value)> {
        self.notify_as(self.server.admin_token(), fake, agent, inputs)
    }
}

fn read_events(res: Response) -> Vec<(String, Value)> {
    use std::io::{BufRead, BufReader};
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

/// Each tool result the model got back in the last request: its error flag and its content as
/// JSON.
fn last_results(fake: &FakeAnthropic) -> Vec<(bool, Value)> {
    let requests = fake.message_requests();
    let r = requests.last().unwrap().json();
    r["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "tool_result")
        .map(|c| {
            (
                c["is_error"] == true,
                serde_json::from_str(c["content"].as_str().unwrap()).unwrap(),
            )
        })
        .collect()
}

/// The one tool result of the last request, after checking its error flag.
fn last_result(fake: &FakeAnthropic, is_error: bool) -> Value {
    let mut results = last_results(fake);
    assert_eq!(results.len(), 1, "{results:?}");
    let (error, content) = results.remove(0);
    assert_eq!(error, is_error, "{content}");
    content
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

fn hello(title: &str) -> Value {
    json!({ "title": title, "body": "", "note_id": "" })
}

#[test]
fn notify_user_stores_a_notification_from_the_agent() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let memo = api.note("memo", json!({ "title": "Roof", "status": "open" }));

    let events = api.notify(
        &fake,
        None,
        &[json!({ "title": "  The roof needs you  ", "body": "Quote due Friday.", "note_id": memo })],
    );
    assert!(tools_in(&fake, 0).contains(&"notify_user".to_owned()));
    let r = last_result(&fake, false);
    assert_eq!(r["sent"], true, "{r}");
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:?}");
    let n = &items[0];
    assert_eq!(n["id"], r["id"]);
    assert_eq!(n["title"], "The roof needs you");
    assert_eq!(n["body"], "Quote due Friday.");
    assert_eq!(n["note"], memo.as_str(), "{n}");
    assert_eq!(n["source"], "agent:assistant");
    assert!(n["url"].is_null(), "{n}");
    // The chat's step names the note it opens.
    let summary = &events
        .iter()
        .rev()
        .find(|(name, _)| name == "tool_result")
        .unwrap()
        .1["summary"];
    assert_eq!(summary["notes"][0]["id"], memo.as_str(), "{summary}");
    assert_eq!(summary["notes"][0]["title"], "Roof");
}

#[test]
fn notify_user_refuses_bad_calls_and_sends_nothing() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("memos_only", MEMOS_ONLY);
    api.agent("reader", READER);
    let person = api.note("person", json!({ "title": "Ann Lee" }));

    let refused = |agent: Option<&str>, input: Value, says: &str| {
        api.notify(&fake, agent, &[input]);
        let r = last_result(&fake, true);
        let error = r["error"].as_str().unwrap();
        assert!(error.contains(says), "{error:?} lacks {says:?}");
    };
    refused(None, hello("   "), "title");
    refused(None, hello(&"x".repeat(201)), "200 characters");
    refused(
        None,
        json!({ "title": "Hi", "body": "y".repeat(2001), "note_id": "" }),
        "2000 characters",
    );
    refused(
        None,
        json!({ "title": "Hi", "body": "", "note_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV" }),
        "no note",
    );
    refused(
        Some("memos_only"),
        json!({ "title": "Hi", "body": "", "note_id": person }),
        "may not read person notes",
    );

    // A caller that may only read cannot notify.
    let viewer = api.token_of("viewer", "read");
    api.notify_as(&viewer, &fake, None, &[hello("Hi")]);
    let r = last_result(&fake, true);
    assert!(r["error"].as_str().unwrap().contains("write"), "{r}");

    // An agent whose tools list leaves it out is not offered it, and a call is not run.
    let n = fake.message_requests().len();
    api.notify(&fake, Some("reader"), &[hello("Hi")]);
    assert!(!tools_in(&fake, n).contains(&"notify_user".to_owned()));
    let r = last_result(&fake, true);
    assert!(r["error"].as_str().unwrap().contains("no tool"), "{r}");

    assert!(api.notifications().is_empty());
}

#[test]
fn notify_user_keeps_to_its_turn_hour_and_day_limits() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);

    // Four in one turn: the fourth is refused.
    api.notify(
        &fake,
        None,
        &[hello("1"), hello("2"), hello("3"), hello("4")],
    );
    let results = last_results(&fake);
    assert_eq!(results.len(), 4, "{results:?}");
    assert!(results[..3].iter().all(|(e, _)| !e), "{results:?}");
    let (error, r) = &results[3];
    assert!(error);
    assert_eq!(r["limited"], true, "{r}");
    assert!(r["error"].as_str().unwrap().contains("this turn"), "{r}");
    assert_eq!(api.notifications().len(), 3);

    // Ten in an hour, from any number of turns.
    for _ in 0..2 {
        api.notify(&fake, None, &[hello("a"), hello("b"), hello("c")]);
    }
    api.notify(&fake, None, &[hello("10"), hello("11")]);
    let results = last_results(&fake);
    assert!(!results[0].0, "{results:?}");
    let (error, r) = &results[1];
    assert!(error);
    assert!(r["error"].as_str().unwrap().contains("last hour"), "{r}");
    assert_eq!(api.notifications().len(), 10);
    // Another agent has its own count.
    api.agent("memos_only", MEMOS_ONLY);
    api.notify(&fake, Some("memos_only"), &[hello("other")]);
    last_result(&fake, false);
    assert_eq!(api.notifications().len(), 11);

    // The next hour lets ten more through, and the third hour ten more: thirty in a day.
    for _ in 0..2 {
        api.advance("PT61M");
        for _ in 0..3 {
            api.notify(&fake, None, &[hello("a"), hello("b"), hello("c")]);
        }
        api.notify(&fake, None, &[hello("j")]);
        last_result(&fake, false);
    }
    api.advance("PT61M");
    api.notify(&fake, None, &[hello("31")]);
    let r = last_result(&fake, true);
    assert!(r["error"].as_str().unwrap().contains("24 hours"), "{r}");
    let from_assistant = api
        .notifications()
        .iter()
        .filter(|n| n["source"] == "agent:assistant")
        .count();
    assert_eq!(from_assistant, 30);

    // A day after the first, the agent may notify again.
    api.advance("PT22H");
    api.notify(&fake, None, &[hello("again")]);
    last_result(&fake, false);
}

#[test]
fn an_unattended_run_is_offered_notify_user_and_uses_it() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    api.agent("watcher", WATCHER);
    let runs = api.get("/api/agents/watcher/runs");
    let tools: Vec<&str> = runs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(tools.contains(&"notify_user"), "{tools:?}");
    // Every confirmation is on, so no change tool is, but notifying is not one.
    assert!(!tools.contains(&"propose_edit"), "{tools:?}");

    let memo = api.note("memo", json!({ "title": "Leak", "status": "open" }));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_n",
        "notify_user",
        json!({ "title": "A leak was reported", "body": "See the memo.", "note_id": memo }),
    ));
    fake.push(AnthropicMessage::text("Told the user."));
    api.advance("PT31S");
    assert_eq!(fake.message_requests().len(), 2);
    assert!(tools_in(&fake, 0).contains(&"notify_user".to_owned()));
    last_result(&fake, false);
    let items = api.notifications();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["title"], "A leak was reported");
    assert_eq!(items[0]["source"], "agent:watcher");
    assert_eq!(items[0]["note"], memo.as_str());
    let runs = api.get("/api/agents/watcher/runs");
    assert_eq!(
        runs["triggers"][0]["runs"][0]["outcome"], "succeeded",
        "{runs}"
    );
}
