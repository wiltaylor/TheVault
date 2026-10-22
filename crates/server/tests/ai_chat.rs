//! S1 with the fake providers: `POST /api/ai/chat`, the chat panel's streamed turn with the
//! server-side tool loop, edit proposals, refusals, the round limit and client disconnects
//! (ai D5, D10, D11, D14, D15).

use std::io::{BufRead, BufReader};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic, FakeOpenAi, OpenAiResponse, TestServer};

const MEMO: &str = r#"label = "Memo"

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
const PLAN_MODEL: &str = "gpt-5.5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Api { server, client }
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn secret(&self, name: &str, value: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/secrets/{name}")))
            .bearer_auth(self.token())
            .json(&json!({ "value": value }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn anthropic(&self, fake: &FakeAnthropic) {
        self.secret("ai_anthropic_api_key", fake.api_key());
        self.secret(
            "ai_settings",
            &json!({
                "default_provider": "anthropic",
                "models": { "anthropic": MODEL },
                "endpoints": { "anthropic": fake.base_url() },
            })
            .to_string(),
        );
    }

    /// The ChatGPT plan: a signed-in `ai_chatgpt` whose access token is not the API key, so the
    /// fake treats it as a plan token.
    fn chatgpt(&self, fake: &FakeOpenAi) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        self.secret(
            "ai_chatgpt",
            &json!({
                "client_id": "oaiapp_test",
                "host_id": "urn:uuid:00000000-0000-4000-8000-000000000000",
                "access_token": "plan-access-token",
                "refresh_token": "plan-refresh-token",
                "expires_ms": now + 3_600_000,
                "account": "me@example.com",
            })
            .to_string(),
        );
        self.secret(
            "ai_settings",
            &json!({
                "default_provider": "chatgpt",
                "models": { "chatgpt": PLAN_MODEL },
                "endpoints": { "openai": fake.base_url() },
            })
            .to_string(),
        );
    }

    fn memo(&self, title: &str, body: &str) -> (String, String) {
        let res = self
            .client
            .post(self.server.url("/api/notes?type=memo"))
            .bearer_auth(self.token())
            .json(&json!({ "note": memo_doc(title, body) }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        (
            v["id"].as_str().unwrap().to_owned(),
            v["version"].as_str().unwrap().to_owned(),
        )
    }

    fn note(&self, id: &str) -> Value {
        self.client
            .get(self.server.url(&format!("/api/notes/{id}")))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .json()
            .unwrap()
    }

    fn chat_as(&self, token: &str, body: Value) -> Response {
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        res
    }

    /// One turn, read to the end of its stream.
    fn chat(&self, messages: Value) -> Vec<(String, Value)> {
        let res = self.chat_as(&self.token(), json!({ "messages": messages }));
        read_events(res, |_| false)
    }
}

fn memo_doc(title: &str, body: &str) -> Value {
    json!({
        "fields": { "title": title },
        "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [
            { "kind": "paragraph", "fields": { "text": body } }
        ] }],
    })
}

/// Read SSE frames as `(event, data)` until the stream ends or `stop` says so.
fn read_events(
    res: Response,
    mut stop: impl FnMut(&(String, Value)) -> bool,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    for line in BufReader::new(res).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                let e = (
                    std::mem::take(&mut name),
                    serde_json::from_str(&data).unwrap(),
                );
                data.clear();
                let done = stop(&e);
                out.push(e);
                if done {
                    return out;
                }
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

fn names(events: &[(String, Value)]) -> Vec<&str> {
    events
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| *n != "usage")
        .collect()
}

/// The event names with each run of `text` events folded into one.
fn shape(events: &[(String, Value)]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for n in names(events) {
        if n == "text" && out.last() == Some(&"text") {
            continue;
        }
        out.push(n);
    }
    out
}

fn text_of(events: &[(String, Value)]) -> String {
    events
        .iter()
        .filter(|(n, _)| n == "text")
        .map(|(_, d)| d["delta"].as_str().unwrap())
        .collect()
}

fn last<'a>(events: &'a [(String, Value)], name: &str) -> &'a Value {
    &events
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
        .1
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

/// The history the client sends next: what it sent, the turn's new items, the next question.
fn next_history(sent: &Value, done: &Value, question: &str) -> Value {
    let mut h = sent.as_array().unwrap().clone();
    h.extend(done["messages"].as_array().unwrap().iter().cloned());
    h.push(user(question));
    Value::Array(h)
}

#[test]
fn an_anthropic_turn_searches_reads_answers_and_replays_unedited() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, _) = api.memo("Roof decision", "We chose slate tiles for the roof.");

    fake.push(
        AnthropicMessage::new()
            .thinking("Search for the roof.", "sig-round-1")
            .tool_use(
                "toolu_1",
                "search_notes",
                json!({ "text": "roof", "limit": 5 }),
            ),
    );
    fake.push(AnthropicMessage::new().tool_use("toolu_2", "get_note", json!({ "id": id })));
    fake.push(AnthropicMessage::text(&format!(
        "You chose slate, see [Roof decision](/notes/{id})."
    )));

    // A read token may chat (ai D15).
    let res = api
        .client
        .post(api.server.url("/api/tokens"))
        .bearer_auth(api.token())
        .json(&json!({ "name": "reader", "scope": "read" }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let reader: Value = res.json().unwrap();
    let sent = json!([user("What did I decide about the roof?")]);
    let events = read_events(
        api.chat_as(
            reader["secret"].as_str().unwrap(),
            json!({ "messages": sent, "note_id": id }),
        ),
        |_| false,
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool")["name"], "get_note");
    assert_eq!(
        events.iter().find(|(n, _)| n == "tool").unwrap().1["name"],
        "search_notes"
    );
    assert!(text_of(&events).starts_with("You chose slate"));
    // Summaries hold ids and titles, never bodies.
    for (n, d) in &events {
        if n == "tool_result" {
            assert_eq!(d["is_error"], false, "{d}");
            assert_eq!(d["summary"]["notes"][0]["id"], id.as_str(), "{d}");
            assert_eq!(d["summary"]["notes"][0]["title"], "Roof decision");
            assert!(!d.to_string().contains("slate"), "{d}");
        }
    }
    let requests = fake.message_requests();
    assert_eq!(requests.len(), 3);
    let first = requests[0].json();
    assert!(
        first["system"].as_str().unwrap().contains(&id),
        "the open note is named in the system prompt"
    );
    assert!(
        first["system"]
            .as_str()
            .unwrap()
            .starts_with(ai::CHAT_PROMPT)
    );
    assert_eq!(first["tools"].as_array().unwrap().len(), 5);
    // The tool result went back with the body.
    let third = requests[2].json();
    assert!(third.to_string().contains("slate tiles"));

    // Replay: the next request starts with exactly what the last one sent, then the answer.
    let done = last(&events, "done");
    assert_eq!(done["messages"].as_array().unwrap().len(), 5);
    fake.push(AnthropicMessage::text("Slate is durable."));
    let history = next_history(&sent, done, "Why slate?");
    let more = api.chat(history);
    assert_eq!(shape(&more), ["text", "done"]);
    let requests = fake.message_requests();
    let replay = requests[3].json();
    let before = third["messages"].as_array().unwrap();
    let after = replay["messages"].as_array().unwrap();
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(after.len(), before.len() + 2);
    // Round one's thinking block went back with its signature, before its tool call.
    let round1 = &after[1]["content"];
    assert_eq!(round1[0]["type"], "thinking");
    assert_eq!(round1[0]["signature"], "sig-round-1");
    assert_eq!(round1[1]["id"], "toolu_1");
}

#[test]
fn a_chatgpt_plan_turn_searches_reads_answers_and_replays_unedited() {
    let api = Api::start();
    let fake = FakeOpenAi::start();
    api.chatgpt(&fake);
    let (id, _) = api.memo("Boiler service", "The boiler was serviced in May.");

    fake.push(
        OpenAiResponse::new()
            .reasoning("Look it up.", "enc-round-1")
            .function_call(
                "call_1",
                "search_notes",
                json!({ "text": "boiler", "limit": 3 }),
            ),
    );
    fake.push(OpenAiResponse::new().function_call("call_2", "get_note", json!({ "id": id })));
    fake.push(OpenAiResponse::text("It was serviced in May."));
    let sent = json!([user("When was the boiler serviced?")]);
    let events = api.chat(sent.clone());
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(text_of(&events), "It was serviced in May.");
    let requests = fake.response_requests();
    assert_eq!(requests.len(), 3);
    // Plan requests: the bearer is the plan token, and the rules hold (the fake checks them).
    assert_eq!(
        requests[0].header("authorization"),
        Some("Bearer plan-access-token")
    );
    let third = requests[2].json();
    assert_eq!(third["store"], false);
    assert!(third.to_string().contains("serviced in May"));

    let done = last(&events, "done");
    fake.push(OpenAiResponse::text("Every year."));
    let more = api.chat(next_history(&sent, done, "How often?"));
    assert_eq!(shape(&more), ["text", "done"]);
    let replay = fake.response_requests()[3].json();
    let before = third["input"].as_array().unwrap();
    let after = replay["input"].as_array().unwrap();
    assert_eq!(&after[..before.len()], before.as_slice());
    let reasoning = after
        .iter()
        .find(|i| i["type"] == "reasoning")
        .expect("the reasoning item is replayed");
    assert_eq!(reasoning["encrypted_content"], "enc-round-1");
    assert!(
        after
            .iter()
            .any(|i| i["type"] == "function_call" && i["call_id"] == "call_1")
    );
}

#[test]
fn propose_edit_emits_a_proposal_and_writes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, version) = api.memo("Groceries", "Milk");
    let proposed = memo_doc("Groceries", "Milk and eggs");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_p",
        "propose_edit",
        json!({ "id": id, "base_version": version, "note": proposed.to_string() }),
    ));
    fake.push(AnthropicMessage::text(
        "I proposed adding eggs; apply it if you agree.",
    ));
    let events = api.chat(json!([user("Add eggs to my groceries.")]));
    assert_eq!(
        shape(&events),
        ["tool", "proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "proposal");
    assert_eq!(p["note_id"], id.as_str());
    assert_eq!(p["base_version"], version.as_str());
    assert_eq!(p["id"], "toolu_p");
    assert_eq!(p["ops"].as_array().unwrap().len(), 1, "{p}");
    assert_eq!(p["ops"][0]["name"], "content body");
    assert!(p["after"].to_string().contains("Milk and eggs"));
    assert!(p["before"].to_string().contains("\"Milk\""));
    // Nothing was written.
    let note = api.note(&id);
    assert_eq!(note["version"], version.as_str());
    assert!(!note.to_string().contains("eggs"));
    // The model is told the edit awaits the user.
    let second = fake.message_requests()[1].json();
    let result = &second["messages"][2]["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["is_error"], false);
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains("awaits the user")
    );
}

#[test]
fn a_proposal_on_a_stale_version_reports_the_conflict_to_the_model() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, old) = api.memo("Plan", "One");
    let res = api
        .client
        .put(api.server.url(&format!("/api/notes/{id}")))
        .bearer_auth(api.token())
        .json(&json!({ "note": memo_doc("Plan", "Two"), "base_version": old }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let current: Value = res.json().unwrap();
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_c",
        "propose_edit",
        json!({ "id": id, "base_version": old, "note": memo_doc("Plan", "Three").to_string() }),
    ));
    fake.push(AnthropicMessage::text(
        "The note changed; let me read it again.",
    ));
    let events = api.chat(json!([user("Make it three.")]));
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    let second = fake.message_requests()[1].json();
    let result = &second["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
    let text = result["content"].as_str().unwrap();
    assert!(text.contains("conflict"), "{text}");
    assert!(
        text.contains(current["version"].as_str().unwrap()),
        "{text}"
    );
    assert_eq!(api.note(&id)["version"], current["version"]);
}

#[test]
fn a_tool_call_with_a_bad_input_is_answered_as_an_error_not_run() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    fake.push(AnthropicMessage::new().tool_use("toolu_x", "get_note", json!({ "nope": 1 })));
    fake.push(AnthropicMessage::text("Sorry."));
    let events = api.chat(json!([user("Read it.")]));
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    let second = fake.message_requests()[1].json();
    assert_eq!(second["messages"][2]["content"][0]["is_error"], true);
}

#[test]
fn a_refusal_ends_the_stream_with_the_model_and_category() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    fake.push(
        AnthropicMessage::text("Here is the start")
            .refusal(Some("cyber"), Some("This touches on attacks.")),
    );
    let events = api.chat(json!([user("Something risky")]));
    let (name, r) = events.last().unwrap();
    assert_eq!(name, "refusal", "{events:?}");
    assert_eq!(r["model"], MODEL);
    assert_eq!(r["provider"], "anthropic");
    assert_eq!(r["category"], "cyber");
    assert_eq!(r["explanation"], "This touches on attacks.");
    assert_eq!(text_of(&events), "Here is the start");
    // The partial answer stays in the transcript, marked cut off.
    let content = &r["messages"][0]["content"];
    assert_eq!(content[0]["text"], "Here is the start");
    assert_eq!(content[1]["type"], "notice");
    assert_eq!(content[1]["kind"], "refusal");
    assert!(!events.iter().any(|(n, _)| n == "done"));
}

#[test]
fn the_thirteenth_tool_round_is_not_sent() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.memo("Anything", "Text");
    for n in 0..13 {
        fake.push(AnthropicMessage::new().tool_use(
            &format!("toolu_{n}"),
            "search_notes",
            json!({ "text": "anything", "limit": 1 }),
        ));
    }
    let events = api.chat(json!([user("Keep looking.")]));
    assert_eq!(fake.message_requests().len(), 12);
    let tools = events.iter().filter(|(n, _)| n == "tool").count();
    assert_eq!(tools, 11, "the twelfth round's calls are not run");
    let (name, done) = events.last().unwrap();
    assert_eq!(name, "done");
    let added = done["messages"].as_array().unwrap();
    assert_eq!(added.len(), 24);
    let tail = &added[23]["content"];
    assert_eq!(tail[0]["type"], "tool_result");
    assert_eq!(tail[0]["is_error"], true);
    assert_eq!(tail[1]["kind"], "round_limit");
}

#[test]
fn a_client_disconnect_stops_further_provider_requests() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    // A slow first round, then rounds that would follow if the loop went on.
    fake.push(
        AnthropicMessage::text(&"slow ".repeat(60))
            .tool_use(
                "toolu_1",
                "search_notes",
                json!({ "text": "x", "limit": 1 }),
            )
            .delay(Duration::from_millis(20)),
    );
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_2",
        "search_notes",
        json!({ "text": "x", "limit": 1 }),
    ));
    fake.push(AnthropicMessage::text("never"));
    let res = api.chat_as(&api.token(), json!({ "messages": [user("Go")] }));
    let got = read_events(res, |(n, _)| n == "text");
    assert_eq!(got.last().unwrap().0, "text");
    // The response is dropped here, closing the connection, well before round one ends.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(fake.message_requests().len(), 1);
}

#[test]
fn an_unconfigured_provider_answers_an_error_event() {
    let api = Api::start();
    let events = api.chat(json!([user("Hello")]));
    let (name, e) = events.last().unwrap();
    assert_eq!(name, "error", "{events:?}");
    assert_eq!(e["kind"], "not_configured");
}
