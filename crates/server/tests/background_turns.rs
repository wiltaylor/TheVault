//! S1 with the fake Anthropic: chat turns on agent threads run in the background (agents G17).
//! A turn runs on after its only reader goes and lands in its thread; any number of readers read
//! it, each from the event it chooses; the thread list and the agent list say what runs; the
//! live stream announces turns starting and ending; Stop ends one; a finished turn's events are
//! kept a while, then answer 410; and an agent runs at most four turns at once.

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AiSeed, AnthropicMessage, FakeAnthropic, TestAccount, TestServer};

const MODEL: &str = "claude-opus-5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start(fake: &FakeAnthropic) -> Api {
        Api::start_with_env(fake, &[])
    }

    fn start_with_env(fake: &FakeAnthropic, env: &[(&str, &str)]) -> Api {
        let server = TestServer::start_with_env(|_| {}, env);
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

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn token_of(&self, name: &str) -> String {
        let v = self.send(
            "POST",
            "/api/tokens",
            json!({ "name": name, "scope": "read" }),
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

    fn send(&self, method: &str, path: &str, body: Value) -> Value {
        let res = self.request_as(&self.token(), method, path, Some(body));
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.text().unwrap()
        );
        res.json().unwrap_or(Value::Null)
    }

    fn get(&self, path: &str) -> Value {
        let res = self.request_as(&self.token(), "GET", path, None);
        assert!(res.status().is_success(), "GET {path}");
        res.json().unwrap()
    }

    fn thread(&self) -> String {
        let v = self.send("POST", "/api/agents/assistant/threads", json!({}));
        v["id"].as_str().unwrap().to_owned()
    }

    /// Start a turn on `thread` as `token`: the status and the answer.
    fn start_turn_as(&self, token: &str, thread: &str, text: &str) -> (StatusCode, Value) {
        let res = self.request_as(
            token,
            "POST",
            "/api/ai/chat/turns",
            Some(json!({ "agent": "assistant", "thread": thread, "messages": [user(text)] })),
        );
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    fn start_turn(&self, thread: &str, text: &str) -> String {
        let (status, v) = self.start_turn_as(&self.token(), thread, text);
        assert_eq!(status, StatusCode::CREATED, "{v}");
        assert!(v["started_ms"].is_i64(), "{v}");
        v["turn"].as_str().unwrap().to_owned()
    }

    fn events(&self, turn: &str, after: Option<usize>) -> Response {
        let q = after.map(|n| format!("?after={n}")).unwrap_or_default();
        self.request_as(
            &self.token(),
            "GET",
            &format!("/api/ai/chat/turns/{turn}/events{q}"),
            None,
        )
    }

    /// Thread `id` as the agent's thread list has it.
    fn listed(&self, id: &str) -> Value {
        let list = self.get("/api/agents/assistant/threads");
        list["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("thread {id} is not listed: {list}"))
    }

    /// Wait until thread `id` no longer runs.
    fn wait_until_idle(&self, id: &str) {
        let until = Instant::now() + Duration::from_secs(30);
        while self.listed(id)["running"] == true {
            assert!(Instant::now() < until, "the turn on {id} never ended");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// One server-sent event: its `id`, name and data.
type Sse = (Option<String>, String, Value);

/// The events of `res`, until it ends or `stop` says so.
fn read_events(res: Response, mut stop: impl FnMut(&Sse) -> bool) -> Vec<Sse> {
    let mut out = Vec::new();
    let (mut id, mut name, mut data) = (None, String::new(), String::new());
    for line in BufReader::new(res).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                let e = (
                    id.take(),
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
        if let Some(v) = line.strip_prefix("id:") {
            id = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("event:") {
            name = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push_str(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    out
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

/// An answer that streams for about `secs` seconds.
fn slow(secs: u64) -> AnthropicMessage {
    // 75 frames of 4 characters.
    AnthropicMessage::text(&"slow ".repeat(60)).delay(Duration::from_millis(secs * 1000 / 75))
}

#[test]
fn a_thread_turn_runs_on_after_its_only_reader_goes_and_lands_in_its_thread() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread();
    fake.push(
        AnthropicMessage::text("Looking.")
            .tool_use(
                "toolu_1",
                "search_notes",
                json!({ "text": "roof", "limit": 5 }),
            )
            .delay(Duration::from_millis(30)),
    );
    fake.push(AnthropicMessage::text("Nothing about the roof."));
    let turn = api.start_turn(&thread, "Anything on the roof?");
    // The only reader reads one event and goes.
    let first = read_events(api.events(&turn, None), |_| true);
    assert_eq!(first.len(), 1, "{first:?}");
    api.wait_until_idle(&thread);
    assert_eq!(fake.message_requests().len(), 2);
    let t = api.get(&format!("/api/agents/assistant/threads/{thread}"));
    let messages = t["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4, "{t}");
    assert_eq!(
        messages[3]["content"][0]["text"], "Nothing about the roof.",
        "{t}"
    );
    assert_eq!(t["running"], false, "{t}");
    assert_eq!(t["turn"], Value::Null, "{t}");
}

#[test]
fn a_reader_joining_mid_turn_after_n_gets_the_missed_events_then_the_live_ones() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread();
    fake.push(slow(3));
    let turn = api.start_turn(&thread, "Go");
    let first = std::thread::scope(|s| {
        let whole = s.spawn(|| read_events(api.events(&turn, None), |_| false));
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(api.listed(&thread)["running"], true);
        let joined = read_events(api.events(&turn, Some(3)), |_| false);
        let whole = whole.join().unwrap();
        assert_eq!(joined, whole[3..].to_vec());
        whole
    });
    assert!(first.len() > 10, "{first:?}");
    // Every event is numbered from 1.
    for (i, (id, _, _)) in first.iter().enumerate() {
        assert_eq!(id.as_deref(), Some((i + 1).to_string().as_str()));
    }
    assert_eq!(first.last().unwrap().1, "done");
}

#[test]
fn the_thread_list_and_the_agent_list_say_what_runs() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let busy = api.thread();
    let idle = api.thread();
    fake.push(slow(2));
    let turn = api.start_turn(&busy, "Go");

    let b = api.listed(&busy);
    assert_eq!(b["running"], true, "{b}");
    assert_eq!(b["turn"], turn.as_str(), "{b}");
    assert!(b["started_ms"].is_i64(), "{b}");
    let i = api.listed(&idle);
    assert_eq!(i["running"], false, "{i}");
    assert_eq!(i["turn"], Value::Null, "{i}");
    let t = api.get(&format!("/api/agents/assistant/threads/{busy}"));
    assert_eq!(t["running"], true, "{t}");
    let agents = api.get("/api/agents");
    assert_eq!(agents["agents"][0]["name"], "assistant");
    assert_eq!(agents["agents"][0]["running"], 1, "{agents}");
    // A second turn on a running thread is refused.
    let (status, v) = api.start_turn_as(&api.token(), &busy, "Again");
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "thread_busy");

    api.wait_until_idle(&busy);
    assert_eq!(api.listed(&busy)["turn"], Value::Null);
    assert_eq!(api.get("/api/agents")["agents"][0]["running"], 0);
}

#[test]
fn turns_starting_and_ending_are_announced_on_the_live_stream() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread();
    let (opened, is_open) = mpsc::channel();
    let live = std::thread::scope(|s| {
        let reader = s.spawn(|| {
            let res = api.request_as(&api.token(), "GET", "/api/events", None);
            let mut lines = BufReader::new(res).lines();
            // The stream's first comment says it is open.
            lines.next();
            opened.send(()).unwrap();
            let (mut name, mut got) = (String::new(), Vec::new());
            for line in lines {
                let line = line.unwrap();
                if let Some(v) = line.strip_prefix("event:") {
                    name = v.trim().to_owned();
                } else if let Some(v) = line.strip_prefix("data:")
                    && name == "ai_turn"
                {
                    got.push(serde_json::from_str::<Value>(v.trim()).unwrap());
                    if got.len() == 2 {
                        break;
                    }
                }
            }
            got
        });
        is_open.recv_timeout(Duration::from_secs(10)).unwrap();
        fake.push(AnthropicMessage::text("Hello."));
        let turn = api.start_turn(&thread, "Hi");
        read_events(api.events(&turn, None), |_| false);
        (turn, reader.join().unwrap())
    });
    let (turn, got) = live;
    assert_eq!(
        got,
        [
            json!({ "agent": "assistant", "thread": thread, "turn": turn, "running": true }),
            json!({ "agent": "assistant", "thread": thread, "turn": turn, "running": false }),
        ]
    );
}

#[test]
fn stop_ends_a_thread_turn_nobody_reads() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    let thread = api.thread();
    fake.push(
        AnthropicMessage::text(&"slow ".repeat(60))
            .tool_use(
                "toolu_1",
                "search_notes",
                json!({ "text": "x", "limit": 1 }),
            )
            .delay(Duration::from_millis(40)),
    );
    fake.push(AnthropicMessage::text("never"));
    let turn = api.start_turn(&thread, "Go");
    std::thread::sleep(Duration::from_millis(300));
    let res = api.request_as(
        &api.token(),
        "DELETE",
        &format!("/api/ai/chat/turns/{turn}"),
        None,
    );
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let stopped = Instant::now();
    api.wait_until_idle(&thread);
    assert!(stopped.elapsed() < Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fake.message_requests().len(), 1);
    assert_eq!(api.events(&turn, None).status(), StatusCode::NOT_FOUND);
    // The thread takes the next turn.
    fake.push(AnthropicMessage::text("Back."));
    let next = api.start_turn(&thread, "Again");
    let events = read_events(api.events(&next, None), |_| false);
    assert_eq!(events.last().unwrap().1, "done", "{events:?}");
}

#[test]
fn a_finished_turns_events_answer_410_once_they_expire() {
    let fake = FakeAnthropic::start();
    let api = Api::start_with_env(&fake, &[("VAULT_TEST_AI_TURN_KEEP_MS", "1500")]);
    let thread = api.thread();
    fake.push(AnthropicMessage::text("Hello."));
    let turn = api.start_turn(&thread, "Hi");
    let first = read_events(api.events(&turn, None), |_| false);
    assert_eq!(first.last().unwrap().1, "done");
    // Kept a while: read again, it replays.
    assert_eq!(read_events(api.events(&turn, None), |_| false), first);
    std::thread::sleep(Duration::from_millis(2500));
    let res = api.events(&turn, None);
    assert_eq!(res.status(), StatusCode::GONE);
    let v: Value = res.json().unwrap();
    assert_eq!(v["error"], "turn_expired", "{v}");
}

#[test]
fn an_agent_runs_at_most_four_turns_across_its_threads() {
    let fake = FakeAnthropic::start();
    let api = Api::start(&fake);
    for _ in 0..4 {
        fake.push(slow(4));
    }
    let (one, two) = (api.token_of("one"), api.token_of("two"));
    for token in [&one, &one, &two, &two] {
        let (status, v) = api.start_turn_as(token, &api.thread(), "Go");
        assert_eq!(status, StatusCode::CREATED, "{v}");
    }
    // A caller with no turn of its own still finds the agent busy.
    let (status, v) = api.start_turn_as(&api.token(), &api.thread(), "Go");
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
    assert_eq!(v["error"], "too_many_turns");
}
