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

field files {
  kind     = "asset"
  multiple = true
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
                "endpoints": { "chatgpt": fake.base_url() },
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
    assert_eq!(first["tools"].as_array().unwrap().len(), 32);
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
fn a_chat_completions_turn_searches_answers_and_replays_unedited() {
    let api = Api::start();
    let fake = FakeOpenAi::builder().chat_completions().start();
    api.secret("ai_openai_api_key", fake.api_key());
    api.secret(
        "ai_settings",
        &json!({
            "default_provider": "openai",
            "models": { "openai": "glm-5.3-flash" },
            "endpoints": { "openai": fake.base_url(), "openai_api": "chat_completions" },
        })
        .to_string(),
    );
    api.memo("Boiler service", "The boiler was serviced in May.");

    fake.push(
        OpenAiResponse::new()
            .function_call(
                "call_1",
                "search_notes",
                json!({ "text": "boiler", "limit": 3 }),
            )
            .chunk(4),
    );
    fake.push(OpenAiResponse::text("It was serviced in May."));
    let sent = json!([user("When was the boiler serviced?")]);
    let events = api.chat(sent.clone());
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(text_of(&events), "It was serviced in May.");
    assert!(fake.response_requests().is_empty());
    let requests = fake.chat_requests();
    assert_eq!(requests.len(), 2);
    let second = requests[1].json();
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    let call = messages
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("the tool call is sent back");
    assert_eq!(call["tool_calls"][0]["id"], "call_1");
    assert_eq!(call["tool_calls"][0]["function"]["name"], "search_notes");
    let result = messages
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("the tool result");
    assert_eq!(result["tool_call_id"], "call_1");
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains("Boiler service"),
        "{result}"
    );
    for t in second["tools"].as_array().unwrap() {
        assert!(t["function"].get("strict").is_none(), "{t}");
    }

    // The next turn replays the history as it was sent.
    let done = last(&events, "done");
    fake.push(OpenAiResponse::text("Every year."));
    let more = api.chat(next_history(&sent, done, "How often?"));
    assert_eq!(shape(&more), ["text", "done"]);
    let replay = fake.chat_requests()[2].json();
    let after = replay["messages"].as_array().unwrap();
    assert_eq!(&after[..messages.len()], messages.as_slice());
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

impl Api {
    fn upload(&self, name: &str, bytes: Vec<u8>) -> String {
        let res = self
            .client
            .post(self.server.url(&format!("/api/assets?filename={name}")))
            .bearer_auth(self.token())
            .body(bytes)
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        v["id"].as_str().unwrap().to_owned()
    }

    /// A memo holding `files`.
    fn memo_with(&self, title: &str, files: &[&str]) -> String {
        let mut doc = memo_doc(title, "See the files.");
        doc["fields"]["files"] = json!(files);
        let res = self
            .client
            .post(self.server.url("/api/notes?type=memo"))
            .bearer_auth(self.token())
            .json(&json!({ "note": doc }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED, "{}", res.text().unwrap());
        let v: Value = res.json().unwrap();
        v["id"].as_str().unwrap().to_owned()
    }
}

/// A PNG of one colour.
fn png(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The JSON a tool result sent the model, by its call id, from an Anthropic request body.
fn anthropic_result(body: &Value, call: &str) -> Value {
    for m in body["messages"].as_array().unwrap() {
        for c in m["content"].as_array().unwrap() {
            if c["type"] == "tool_result" && c["tool_use_id"] == call {
                return c.clone();
            }
        }
    }
    panic!("no result for {call} in {body}");
}

/// The text of a tool result: its string content, or its first text block.
fn result_text(result: &Value) -> Value {
    let text = match &result["content"] {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks[0]["text"].as_str().unwrap().to_owned(),
        other => panic!("{other}"),
    };
    serde_json::from_str(&text).unwrap()
}

#[test]
fn read_asset_sends_an_image_to_the_provider_but_never_to_the_client() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let bytes = png(64, 48);
    let asset = api.upload("photo.png", bytes.clone());
    let note = api.memo_with("Holiday", &[&asset]);

    fake.push(AnthropicMessage::new().tool_use("toolu_1", "get_note", json!({ "id": note })));
    fake.push(AnthropicMessage::new().tool_use("toolu_2", "read_asset", json!({ "id": asset })));
    fake.push(AnthropicMessage::text("A blue picture."));
    let sent = json!([user("What is in the holiday photo?")]);
    let events = api.chat(sent.clone());
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    let requests = fake.message_requests();
    // get_note lists the note's assets for read_asset.
    let got = result_text(&anthropic_result(&requests[1].json(), "toolu_1"));
    assert_eq!(got["assets"][0]["id"], asset.as_str(), "{got}");
    assert_eq!(got["assets"][0]["media_type"], "image/png");
    // The image went to the provider, as it is, in the tool result.
    let images = fake.images();
    assert_eq!(images[2].len(), 1, "{images:?}");
    assert_eq!(images[2][0].tool_call_id, "toolu_2");
    assert_eq!(images[2][0].media_type, "image/png");
    assert_eq!(images[2][0].bytes, bytes);
    // The client saw the asset's metadata, never its bytes.
    let summary = &last(&events, "tool_result")["summary"];
    assert_eq!(summary["asset"]["id"], asset.as_str());
    assert_eq!(summary["asset"]["kind"], "image");
    let stream = format!("{events:?}");
    assert!(
        !stream.contains(&b64(&bytes)),
        "image bytes reached the client"
    );
    assert!(!stream.contains("base64"), "{stream}");
    let done = last(&events, "done");
    let parts = &done["messages"][3]["content"][0]["parts"];
    assert_eq!(
        parts,
        &json!([{ "type": "image", "asset_id": asset, "media_type": "image/png" }])
    );

    // Replaying the turn sends the image again, loaded from the vault.
    fake.push(AnthropicMessage::text("Still blue."));
    let more = api.chat(next_history(&sent, done, "And now?"));
    assert_eq!(shape(&more), ["text", "done"]);
    let images = fake.images();
    assert_eq!(images[3].len(), 1);
    assert_eq!(images[3][0].bytes, bytes);
}

#[test]
fn read_asset_sends_an_image_in_the_chatgpt_function_call_output() {
    let api = Api::start();
    let fake = FakeOpenAi::start();
    api.chatgpt(&fake);
    let bytes = png(20, 20);
    let asset = api.upload("dot.png", bytes.clone());
    fake.push(OpenAiResponse::new().function_call("call_1", "read_asset", json!({ "id": asset })));
    fake.push(OpenAiResponse::text("A dot."));
    let events = api.chat(json!([user("What is it?")]));
    assert_eq!(shape(&events), ["tool", "tool_result", "text", "done"]);
    let images = fake.images();
    assert_eq!(images[1].len(), 1);
    assert_eq!(images[1][0].tool_call_id, "call_1");
    assert_eq!(images[1][0].bytes, bytes);
}

#[test]
fn a_large_image_is_sent_as_a_jpeg_at_1568_pixels() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let asset = api.upload("wide.png", png(4000, 1000));
    fake.push(AnthropicMessage::new().tool_use("toolu_1", "read_asset", json!({ "id": asset })));
    fake.push(AnthropicMessage::text("Wide."));
    api.chat(json!([user("Look.")]));
    let images = fake.images();
    let sent = &images[1][0];
    assert_eq!(sent.media_type, "image/jpeg");
    let img = image::load_from_memory(&sent.bytes).unwrap();
    assert_eq!((img.width(), img.height()), (1568, 392));
    let got = result_text(&anthropic_result(
        &fake.message_requests()[1].json(),
        "toolu_1",
    ));
    assert_eq!(got["sent_as"], "image/jpeg");
}

#[test]
fn read_asset_returns_a_pdfs_text_and_metadata_for_other_files() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let pdf = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../index/tests/fixtures/three-pages.pdf"
    ))
    .unwrap();
    let pdf = api.upload("report.pdf", pdf);
    let broken = api.upload(
        "broken.pdf",
        b"%PDF-1.7\n1 0 obj << /Pages 9 0 R >>\n%%EOF".to_vec(),
    );
    let blob = api.upload("data.bin", vec![0, 1, 2, 3, 4]);
    let notes = api.upload("notes.md", b"# Plan\n\nBuy slate.".to_vec());
    fake.push(
        AnthropicMessage::new()
            .tool_use("toolu_pdf", "read_asset", json!({ "id": pdf }))
            .tool_use("toolu_bad", "read_asset", json!({ "id": broken }))
            .tool_use("toolu_bin", "read_asset", json!({ "id": blob }))
            .tool_use("toolu_md", "read_asset", json!({ "id": notes })),
    );
    fake.push(AnthropicMessage::text("Read them."));
    let events = api.chat(json!([user("Read the files.")]));
    assert_eq!(fake.images()[1].len(), 0);
    let body = fake.message_requests()[1].json();

    let got = result_text(&anthropic_result(&body, "toolu_pdf"));
    assert!(got["text"].as_str().unwrap().contains("zephyrine"), "{got}");
    let got = result_text(&anthropic_result(&body, "toolu_bad"));
    assert!(got.get("text").is_none(), "{got}");
    assert!(
        got["note"].as_str().unwrap().contains("unreadable"),
        "{got}"
    );
    let got = result_text(&anthropic_result(&body, "toolu_bin"));
    assert_eq!(
        (
            &got["id"],
            &got["filename"],
            &got["media_type"],
            &got["size"]
        ),
        (
            &json!(blob),
            &json!("data.bin"),
            &json!("application/octet-stream"),
            &json!(5)
        )
    );
    assert!(got.get("text").is_none());
    let got = result_text(&anthropic_result(&body, "toolu_md"));
    assert_eq!(got["text"], "# Plan\n\nBuy slate.");
    // Summaries carry metadata, never the text.
    for (n, d) in &events {
        if n == "tool_result" {
            assert_eq!(d["is_error"], false, "{d}");
            assert!(!d.to_string().contains("zephyrine"), "{d}");
            assert!(!d.to_string().contains("slate"), "{d}");
        }
    }
}

#[test]
fn the_ninth_image_in_a_turn_is_refused() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let assets: Vec<String> = (0..9)
        .map(|n| api.upload(&format!("p{n}.png"), png(8 + n, 8)))
        .collect();
    let mut first = AnthropicMessage::new();
    for (n, a) in assets.iter().take(5).enumerate() {
        first = first.tool_use(&format!("toolu_{n}"), "read_asset", json!({ "id": a }));
    }
    let mut second = AnthropicMessage::new();
    for (n, a) in assets.iter().enumerate().skip(5) {
        second = second.tool_use(&format!("toolu_{n}"), "read_asset", json!({ "id": a }));
    }
    fake.push(first);
    fake.push(second);
    fake.push(AnthropicMessage::text("Eight of them."));
    let events = api.chat(json!([user("Look at all of them.")]));
    let results: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "tool_result")
        .map(|(_, d)| d)
        .collect();
    assert_eq!(results.len(), 9);
    assert!(results[..8].iter().all(|r| r["is_error"] == false));
    assert_eq!(results[8]["is_error"], true);
    let images = fake.images();
    assert_eq!(images[1].len(), 5);
    assert_eq!(images[2].len(), 8);
    let refused = anthropic_result(&fake.message_requests()[2].json(), "toolu_8");
    assert_eq!(refused["is_error"], true);
    assert!(
        refused["content"]
            .as_str()
            .unwrap()
            .contains("limit of 8 images"),
        "{refused}"
    );
}

#[test]
fn a_deleted_asset_is_replayed_as_a_text_saying_so() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let asset = api.upload("gone.png", png(10, 10));
    fake.push(AnthropicMessage::new().tool_use("toolu_1", "read_asset", json!({ "id": asset })));
    fake.push(AnthropicMessage::text("Seen."));
    let sent = json!([user("Look.")]);
    let events = api.chat(sent.clone());
    let res = api
        .client
        .delete(api.server.url(&format!("/api/assets/{asset}")))
        .bearer_auth(api.token())
        .send()
        .unwrap();
    assert!(res.status().is_success(), "{}", res.text().unwrap());
    fake.push(AnthropicMessage::text("It is gone."));
    api.chat(next_history(&sent, last(&events, "done"), "Again?"));
    assert_eq!(fake.images()[2].len(), 0);
    let result = anthropic_result(&fake.message_requests()[2].json(), "toolu_1");
    assert_eq!(
        result["content"][1],
        json!({ "type": "text", "text": format!("asset {asset} no longer exists") })
    );
}

#[test]
fn an_unconfigured_provider_answers_an_error_event() {
    let api = Api::start();
    let events = api.chat(json!([user("Hello")]));
    let (name, e) = events.last().unwrap();
    assert_eq!(name, "error", "{events:?}");
    assert_eq!(e["kind"], "not_configured");
}

impl Api {
    fn put_settings(&self, body: Value) -> (StatusCode, Value) {
        let res = self
            .client
            .put(self.server.url("/api/integrations/ai/settings"))
            .bearer_auth(self.token())
            .json(&body)
            .send()
            .unwrap();
        (res.status(), res.json().unwrap())
    }

    fn ai_status(&self) -> Value {
        self.client
            .get(self.server.url("/api/integrations/ai"))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .json()
            .unwrap()
    }
}

#[test]
fn the_system_prompt_setting_replaces_the_default_for_every_provider() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, _) = api.memo("Roof decision", "Slate.");

    // No setting: the shipped prompt, as it is.
    let status = api.ai_status();
    assert_eq!(status["system_prompt"], ai::CHAT_PROMPT);
    assert_eq!(status["default_system_prompt"], ai::CHAT_PROMPT);
    assert_eq!(status["system_prompt_custom"], false);
    fake.push(AnthropicMessage::text("Hi."));
    api.chat(json!([user("Hello")]));
    assert_eq!(fake.message_requests()[0].json()["system"], ai::CHAT_PROMPT);

    // A custom prompt, with the open note appended after it.
    let custom = "You are terse. Answer in one line.";
    let (code, status) = api.put_settings(json!({ "system_prompt": custom }));
    assert_eq!(code, StatusCode::OK, "{status}");
    assert_eq!(status["system_prompt"], custom);
    assert_eq!(status["default_system_prompt"], ai::CHAT_PROMPT);
    assert_eq!(status["system_prompt_custom"], true);
    // Other settings are untouched by the prompt.
    assert_eq!(status["default_provider"], "anthropic");
    assert_eq!(status["providers"]["anthropic"]["model"], MODEL);
    fake.push(AnthropicMessage::text("Hi."));
    let res = api.chat_as(
        &api.token(),
        json!({ "messages": [user("Hello")], "note_id": id }),
    );
    read_events(res, |_| false);
    let system = fake.message_requests()[1].json()["system"]
        .as_str()
        .unwrap()
        .to_owned();
    let context = system
        .strip_prefix(custom)
        .expect("the custom prompt comes first");
    assert!(context.contains("## The open note"), "{system}");
    assert!(context.contains(&id), "{system}");
    assert!(!system.contains(ai::CHAT_PROMPT));

    // Over 32 000 characters is refused and the stored prompt stays.
    let (code, err) = api.put_settings(json!({ "system_prompt": "x".repeat(32_001) }));
    assert_eq!(code, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(api.ai_status()["system_prompt"], custom);
    let (code, _) = api.put_settings(json!({ "system_prompt": "é".repeat(32_000) }));
    assert_eq!(
        code,
        StatusCode::OK,
        "the limit counts characters, not bytes"
    );

    // An empty prompt resets to the default.
    let (code, status) = api.put_settings(json!({ "system_prompt": "" }));
    assert_eq!(code, StatusCode::OK);
    assert_eq!(status["system_prompt"], ai::CHAT_PROMPT);
    assert_eq!(status["system_prompt_custom"], false);
    fake.push(AnthropicMessage::text("Hi."));
    api.chat(json!([user("Hello")]));
    assert_eq!(fake.message_requests()[2].json()["system"], ai::CHAT_PROMPT);

    // The OpenAI Responses form carries it as the instructions.
    let openai = FakeOpenAi::start();
    api.chatgpt(&openai);
    api.put_settings(json!({ "system_prompt": custom }));
    openai.push(OpenAiResponse::text("Hi."));
    api.chat(json!([user("Hello")]));
    let instructions = openai.response_requests()[0].json()["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(instructions.starts_with(custom), "{instructions}");
}

impl Api {
    /// `types/<name>.wcl` as the types API reads it, or `None` when there is no such type.
    fn type_wcl(&self, name: &str) -> Option<String> {
        let res = self
            .client
            .get(self.server.url(&format!("/api/types/{name}")))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        if !res.status().is_success() {
            return None;
        }
        let v: Value = res.json().unwrap();
        Some(v["wcl"].as_str().unwrap().to_owned())
    }
}

/// The tool result the model got back in request `request`: the last message's first item.
fn tool_result_in(fake: &FakeAnthropic, request: usize) -> Value {
    let r = fake.message_requests()[request].json();
    let messages = r["messages"].as_array().unwrap();
    messages.last().unwrap()["content"][0].clone()
}

#[test]
fn type_tools_list_read_and_propose_a_template_change_without_writing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.memo("Groceries", "Milk");
    let before = api.type_wcl("memo").unwrap();
    let changed = MEMO.replace("tab \"Main\"", "tab \"Details\"");
    fake.push(AnthropicMessage::new().tool_use("toolu_l", "list_types", json!({})));
    fake.push(AnthropicMessage::new().tool_use("toolu_g", "get_type", json!({ "name": "memo" })));
    fake.push(AnthropicMessage::new().tool_use("toolu_f", "type_format", json!({})));
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_t",
        "propose_type_change",
        json!({ "name": "memo", "wcl": changed }),
    ));
    fake.push(AnthropicMessage::text("I proposed renaming the tab."));
    let events = api.chat(json!([user("Call the memo tab Details.")]));
    assert_eq!(
        shape(&events),
        [
            "tool",
            "tool_result",
            "tool",
            "tool_result",
            "tool",
            "tool_result",
            "tool",
            "type_proposal",
            "tool_result",
            "text",
            "done"
        ],
        "{events:?}"
    );
    for (n, d) in &events {
        if n == "tool_result" {
            assert_eq!(d["is_error"], false, "{d}");
        }
    }

    // list_types names every type, built-in ones marked.
    let listed: Value =
        serde_json::from_str(tool_result_in(&fake, 1)["content"].as_str().unwrap()).unwrap();
    let types = listed["types"].as_array().unwrap();
    let memo = types.iter().find(|t| t["name"] == "memo").unwrap();
    assert_eq!(memo["label"], "Memo");
    assert_eq!(memo["builtin"], false);
    assert!(types.iter().any(|t| t["builtin"] == true), "{listed}");

    // get_type gives the stored WCL and a summary of fields and templates.
    let got: Value =
        serde_json::from_str(tool_result_in(&fake, 2)["content"].as_str().unwrap()).unwrap();
    assert_eq!(got["wcl"], before.as_str());
    assert_eq!(got["summary"]["fields"][0]["name"], "body");
    assert_eq!(got["summary"]["fields"][0]["kind"], "content");
    assert_eq!(got["summary"]["templates"][0]["tabs"][0]["title"], "Main");

    // type_format is the type-file part of docs/vault-format.md.
    let format = tool_result_in(&fake, 3)["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(format.contains("## Type files"), "{}", &format[..200]);
    assert!(format.contains("## Templates"));

    let p = last(&events, "type_proposal");
    assert_eq!(p["id"], "toolu_t");
    assert_eq!(p["name"], "memo");
    assert_eq!(p["exists"], true);
    assert_eq!(p["before"], before.as_str());
    assert!(
        p["after"].as_str().unwrap().contains("tab \"Details\""),
        "{p}"
    );
    assert_eq!(p["issues"], json!([]));

    // The model is told the change awaits the user, and nothing was written.
    let result = tool_result_in(&fake, 4);
    assert_eq!(result["is_error"], false);
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains("awaits the user"),
        "{result}"
    );
    assert_eq!(api.type_wcl("memo").unwrap(), before);
}

#[test]
fn an_invalid_type_proposal_returns_the_issues_and_proposes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let before = api.type_wcl("memo").unwrap();
    let broken = MEMO.replace("kind = \"content\"", "kind = \"nope\"");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_bad",
        "propose_type_change",
        json!({ "name": "memo", "wcl": broken }),
    ));
    fake.push(AnthropicMessage::text("Let me fix that."));
    let events = api.chat(json!([user("Change the memo type.")]));
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    let result = tool_result_in(&fake, 1);
    assert_eq!(result["is_error"], true);
    let content: Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
    assert_eq!(content["issues"]["error"], "invalid_type", "{content}");
    assert!(content.to_string().contains("nope"), "{content}");
    assert_eq!(api.type_wcl("memo").unwrap(), before);
}

#[test]
fn a_new_type_name_proposes_a_new_type() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let book = MEMO.replace("\"Memo\"", "\"Book\"");
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_new",
        "propose_type_change",
        json!({ "name": "book", "wcl": book }),
    ));
    fake.push(AnthropicMessage::text("I proposed a book type."));
    let events = api.chat(json!([user("Add a book type.")]));
    assert_eq!(
        shape(&events),
        ["tool", "type_proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "type_proposal");
    assert_eq!(p["name"], "book");
    assert_eq!(p["exists"], false);
    assert_eq!(p["before"], Value::Null);
    assert!(p["after"].as_str().unwrap().contains("\"Book\""), "{p}");
    let result = tool_result_in(&fake, 1);
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains("awaits the user")
    );
    assert_eq!(api.type_wcl("book"), None);
}

impl Api {
    /// The memo notes listed, archived ones included.
    fn memos(&self) -> Vec<Value> {
        let v: Value = self
            .client
            .get(self.server.url("/api/notes?type=memo&archived=include"))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .json()
            .unwrap();
        v["notes"].as_array().unwrap().clone()
    }

    fn post(&self, path: &str) -> Value {
        let res = self
            .client
            .post(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
        res.json().unwrap()
    }
}

/// One proposal call, then an answer: the turn's events.
fn propose_one(api: &Api, fake: &FakeAnthropic, tool: &str, input: Value) -> Vec<(String, Value)> {
    fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
    fake.push(AnthropicMessage::text("Done."));
    api.chat(json!([user("Do it.")]))
}

/// The tool result the model got, as JSON, after checking its error flag.
fn result_json(fake: &FakeAnthropic, is_error: bool) -> Value {
    let result = tool_result_in(fake, 1);
    assert_eq!(result["is_error"], is_error, "{result}");
    serde_json::from_str(result["content"].as_str().unwrap()).unwrap()
}

#[test]
fn propose_create_emits_a_create_proposal_and_writes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let note = memo_doc("Reading list", "Dune");
    let events = propose_one(
        &api,
        &fake,
        "propose_create",
        json!({ "type": "memo", "note": note.to_string() }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "proposal");
    assert_eq!(p["op"], "create");
    assert_eq!(p["id"], "toolu_x");
    assert_eq!(p["type"], "memo");
    assert_eq!(p["title"], "Reading list");
    assert!(p["after"].to_string().contains("Dune"), "{p}");
    let content = result_json(&fake, false);
    assert!(
        content["message"]
            .as_str()
            .unwrap()
            .contains("awaits the user")
    );
    assert_eq!(api.memos(), Vec::<Value>::new());
}

#[test]
fn an_invalid_create_returns_the_issues_and_proposes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let mut note = memo_doc("Broken", "x");
    note["fields"]["files"] = json!(42);
    let events = propose_one(
        &api,
        &fake,
        "propose_create",
        json!({ "type": "memo", "note": note.to_string() }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    let content = result_json(&fake, true);
    assert!(content["details"]["error"].is_string(), "{content}");
    assert!(
        content["details"].to_string().contains("files"),
        "{content}"
    );
    assert_eq!(api.memos(), Vec::<Value>::new());
}

#[test]
fn propose_archive_emits_archive_and_unarchive_proposals_and_writes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, version) = api.memo("Old plan", "Done");
    let events = propose_one(
        &api,
        &fake,
        "propose_archive",
        json!({ "id": id, "archived": true }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "proposal");
    assert_eq!(p["op"], "archive");
    assert_eq!(p["note_id"], id.as_str());
    assert_eq!(p["title"], "Old plan");
    assert_eq!(p["base_version"], version.as_str());
    let content = result_json(&fake, false);
    assert!(
        content["message"]
            .as_str()
            .unwrap()
            .contains("awaits the user")
    );
    assert_eq!(api.note(&id)["version"], version.as_str());

    // Once archived, archiving again is an error and unarchiving is the proposal.
    let archived = api.post(&format!("/api/notes/{id}/archive?base_version={version}"));
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let events = propose_one(
        &api,
        &fake,
        "propose_archive",
        json!({ "id": id, "archived": true }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    let content = result_json(&fake, true);
    assert!(
        content["error"]
            .as_str()
            .unwrap()
            .contains("already archived"),
        "{content}"
    );
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let events = propose_one(
        &api,
        &fake,
        "propose_archive",
        json!({ "id": id, "archived": false }),
    );
    let p = last(&events, "proposal");
    assert_eq!(p["op"], "unarchive");
    assert_eq!(p["base_version"], archived["version"]);
    assert_eq!(api.note(&id)["version"], archived["version"]);
}

#[test]
fn propose_delete_emits_a_delete_proposal_and_writes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, version) = api.memo("Scratch", "Throwaway");
    let events = propose_one(&api, &fake, "propose_delete", json!({ "id": id }));
    assert_eq!(
        shape(&events),
        ["tool", "proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "proposal");
    assert_eq!(p["op"], "delete");
    assert_eq!(p["note_id"], id.as_str());
    assert_eq!(p["title"], "Scratch");
    assert_eq!(p["base_version"], version.as_str());
    let content = result_json(&fake, false);
    let message = content["message"].as_str().unwrap();
    assert!(message.contains("awaits the user"), "{message}");
    assert!(message.contains("trash"), "{message}");
    assert_eq!(api.note(&id)["version"], version.as_str());
    assert_eq!(api.memos().len(), 1);
}

#[test]
fn deleting_a_missing_note_is_an_error() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let events = propose_one(
        &api,
        &fake,
        "propose_delete",
        json!({ "id": "01JZZZZZZZZZZZZZZZZZZZZZZZ" }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    result_json(&fake, true);
}

#[test]
fn an_edit_proposal_says_it_is_an_update() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let (id, version) = api.memo("Groceries", "Milk");
    let events = propose_one(
        &api,
        &fake,
        "propose_edit",
        json!({ "id": id, "base_version": version, "note": memo_doc("Groceries", "Eggs").to_string() }),
    );
    assert_eq!(last(&events, "proposal")["op"], "update");
}

impl Api {
    /// `PUT /api/integrations/ai/settings` with a partial `confirm` (ai D22): its answer.
    fn confirm(&self, confirm: Value) -> Value {
        let res = self
            .client
            .put(self.server.url("/api/integrations/ai/settings"))
            .bearer_auth(self.token())
            .json(&json!({ "confirm": confirm }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        res.json().unwrap()
    }

    /// A new API token of `scope`, named `name`: its secret.
    fn token_of(&self, name: &str, scope: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/tokens"))
            .bearer_auth(self.token())
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        v["secret"].as_str().unwrap().to_owned()
    }

    fn get(&self, path: &str) -> Value {
        self.client
            .get(self.server.url(path))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .json()
            .unwrap()
    }

    /// One proposal call as `token`, then an answer: the turn's events.
    fn propose_as(
        &self,
        token: &str,
        fake: &FakeAnthropic,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_x", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let res = self.chat_as(token, json!({ "messages": [user("Do it.")] }));
        read_events(res, |_| false)
    }
}

const ALL_OFF: &str = r#"{"notes": false, "archive": false, "delete": false, "types": false}"#;

#[test]
fn confirm_defaults_to_on_and_takes_a_partial_change() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let status = api.get("/api/integrations/ai");
    assert_eq!(
        status["confirm"],
        json!({ "notes": true, "archive": true, "delete": true, "types": true, "downloads": true, "taxonomies": true, "agents": true })
    );
    let status = api.confirm(json!({ "notes": false }));
    assert_eq!(
        status["confirm"],
        json!({ "notes": false, "archive": true, "delete": true, "types": true, "downloads": true, "taxonomies": true, "agents": true })
    );
    let status = api.confirm(json!({ "delete": false }));
    assert_eq!(
        status["confirm"],
        json!({ "notes": false, "archive": true, "delete": false, "types": true, "downloads": true, "taxonomies": true, "agents": true })
    );
    // The other settings are kept.
    assert_eq!(status["default_provider"], "anthropic");
    let res = api
        .client
        .put(api.server.url("/api/integrations/ai/settings"))
        .bearer_auth(api.token())
        .json(&json!({ "confirm": { "purge": false } }))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16() / 100, 4);
}

#[test]
fn with_notes_confirmation_off_edits_and_creates_are_written_as_the_caller() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "notes": false }));
    let agent = api.token_of("agent", "write");
    let (id, version) = api.memo("Groceries", "Milk");
    let events = api.propose_as(
        &agent,
        &fake,
        "propose_edit",
        json!({ "id": id, "base_version": version, "note": memo_doc("Groceries", "Milk and eggs").to_string() }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    let note = api.note(&id);
    assert!(note.to_string().contains("Milk and eggs"), "{note}");
    assert_eq!(a["id"], "toolu_x");
    assert_eq!(a["op"], "update");
    assert_eq!(a["note_id"], id.as_str());
    assert_eq!(a["title"], "Groceries");
    assert_eq!(a["version"], note["version"]);
    assert_ne!(a["version"], version.as_str());
    let content = result_json(&fake, false);
    assert_eq!(content["applied"], true);
    assert_eq!(content["version"], note["version"]);
    assert!(
        content["message"]
            .as_str()
            .unwrap()
            .contains(note["version"].as_str().unwrap()),
        "{content}"
    );
    let history = api.get(&format!("/api/notes/{id}/history"));
    let newest = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["current"] == true)
        .unwrap();
    assert_eq!(
        newest["author"],
        json!({ "kind": "token", "name": "agent" }),
        "{history}"
    );

    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "notes": false }));
    let events = api.propose_as(
        &agent,
        &fake,
        "propose_create",
        json!({ "type": "memo", "note": memo_doc("Reading list", "Dune").to_string() }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    assert_eq!(a["op"], "create");
    assert_eq!(a["title"], "Reading list");
    let created = api.note(a["note_id"].as_str().unwrap());
    assert!(created.to_string().contains("Dune"), "{created}");
    assert_eq!(a["version"], created["version"]);
    let content = result_json(&fake, false);
    assert_eq!(content["note_id"], a["note_id"]);
    assert_eq!(api.memos().len(), 2);
}

#[test]
fn with_notes_confirmation_off_a_version_conflict_writes_nothing() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "notes": false }));
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
    let events = propose_one(
        &api,
        &fake,
        "propose_edit",
        json!({ "id": id, "base_version": old, "note": memo_doc("Plan", "Three").to_string() }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert_eq!(last(&events, "tool_result")["is_error"], true);
    let result = tool_result_in(&fake, 1);
    assert_eq!(result["is_error"], true);
    let text = result["content"].as_str().unwrap();
    assert!(text.contains("conflict"), "{text}");
    assert!(text.contains("Nothing was written"), "{text}");
    let note = api.note(&id);
    assert_eq!(note["version"], current["version"]);
    assert!(!note.to_string().contains("Three"));
}

#[test]
fn with_archive_and_delete_confirmation_off_the_note_is_archived_then_trashed_never_purged() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "archive": false, "delete": false }));
    let (id, version) = api.memo("Scratch", "Throwaway");
    let events = propose_one(
        &api,
        &fake,
        "propose_archive",
        json!({ "id": id, "archived": true }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    assert_eq!(a["op"], "archive");
    let note = api.note(&id);
    assert_eq!(a["version"], note["version"]);
    assert_ne!(note["version"], version.as_str());
    assert_eq!(api.memos()[0]["archived"], true);

    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "archive": false, "delete": false }));
    let events = propose_one(&api, &fake, "propose_delete", json!({ "id": id }));
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    assert_eq!(a["op"], "delete");
    assert_eq!(a["note_id"], id.as_str());
    assert_eq!(a["title"], "Scratch");
    let content = result_json(&fake, false);
    assert!(
        content["message"].as_str().unwrap().contains("trash"),
        "{content}"
    );
    assert_eq!(api.memos(), Vec::<Value>::new());
    let trash = api.get("/api/trash");
    let entries = trash["notes"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{trash}");
    assert_eq!(entries[0]["id"], id.as_str());
    // The trash entry stays restorable: its history is kept.
    let restored = api.post(&format!("/api/trash/{id}/restore"));
    assert_eq!(restored["id"], id.as_str());
}

#[test]
fn with_types_confirmation_off_a_template_change_is_applied_through_the_planned_change() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "types": false }));
    let (id, _) = api.memo("Groceries", "Milk");
    let changed = MEMO.replace("tab \"Main\"", "tab \"Details\"");
    let events = propose_one(
        &api,
        &fake,
        "propose_type_change",
        json!({ "name": "memo", "wcl": changed }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    assert_eq!(a["op"], "type_change");
    assert_eq!(a["type_name"], "memo");
    assert!(
        api.type_wcl("memo").unwrap().contains("tab \"Details\""),
        "{a}"
    );
    assert!(api.note(&id).to_string().contains("Milk"));
    let content = result_json(&fake, false);
    assert_eq!(content["applied"], true);

    // A new type is created through the type write.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(json!({ "types": false }));
    let book = MEMO.replace("\"Memo\"", "\"Book\"");
    let events = propose_one(
        &api,
        &fake,
        "propose_type_change",
        json!({ "name": "book", "wcl": book }),
    );
    assert_eq!(last(&events, "applied")["op"], "type_create");
    assert!(api.type_wcl("book").unwrap().contains("\"Book\""));
}

#[test]
fn a_caller_without_the_scope_gets_proposals_whatever_the_switches_say() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(serde_json::from_str(ALL_OFF).unwrap());
    let reader = api.token_of("reader", "read");
    let (id, version) = api.memo("Groceries", "Milk");
    let calls = [
        (
            "propose_edit",
            json!({ "id": id, "base_version": version, "note": memo_doc("Groceries", "Eggs").to_string() }),
            "proposal",
        ),
        (
            "propose_create",
            json!({ "type": "memo", "note": memo_doc("New", "x").to_string() }),
            "proposal",
        ),
        (
            "propose_archive",
            json!({ "id": id, "archived": true }),
            "proposal",
        ),
        ("propose_delete", json!({ "id": id }), "proposal"),
        (
            "propose_type_change",
            json!({ "name": "memo", "wcl": MEMO.replace("tab \"Main\"", "tab \"Details\"") }),
            "type_proposal",
        ),
    ];
    for (tool, input, event) in calls.clone() {
        let fake = FakeAnthropic::start();
        api.anthropic(&fake);
        api.confirm(serde_json::from_str(ALL_OFF).unwrap());
        let events = api.propose_as(&reader, &fake, tool, input);
        assert_eq!(
            shape(&events),
            ["tool", event, "tool_result", "text", "done"],
            "{tool}: {events:?}"
        );
        assert!(
            result_json(&fake, false)["message"]
                .as_str()
                .unwrap()
                .contains("awaits the user"),
            "{tool}"
        );
    }
    assert_eq!(api.note(&id)["version"], version.as_str());
    assert_eq!(api.memos().len(), 1);
    assert!(api.type_wcl("memo").unwrap().contains("tab \"Main\""));

    // A write token makes note changes but still proposes type changes, which need admin.
    let writer = api.token_of("writer", "write");
    let (tool, input, _) = calls[4].clone();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.confirm(serde_json::from_str(ALL_OFF).unwrap());
    let events = api.propose_as(&writer, &fake, tool, input);
    assert_eq!(
        shape(&events),
        ["tool", "type_proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    assert!(api.type_wcl("memo").unwrap().contains("tab \"Main\""));
}

// The turns form and the apply route (ai D23).

impl Api {
    /// `POST /api/ai/chat/turns` as `token`: the answer's status and body.
    fn start_turn(&self, token: &str, body: Value) -> (StatusCode, Value) {
        let res = self
            .client
            .post(self.server.url("/api/ai/chat/turns"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }

    /// A turn started as the admin: its id.
    fn turn(&self, messages: Value) -> String {
        let (status, v) = self.start_turn(&self.token(), json!({ "messages": messages }));
        assert_eq!(status, StatusCode::CREATED, "{v}");
        v["turn"].as_str().unwrap().to_owned()
    }

    fn turn_events(&self, token: &str, turn: &str) -> Response {
        self.client
            .get(
                self.server
                    .url(&format!("/api/ai/chat/turns/{turn}/events")),
            )
            .bearer_auth(token)
            .send()
            .unwrap()
    }

    fn stop_turn(&self, token: &str, turn: &str) -> StatusCode {
        self.client
            .delete(self.server.url(&format!("/api/ai/chat/turns/{turn}")))
            .bearer_auth(token)
            .send()
            .unwrap()
            .status()
    }

    /// `POST /api/ai/apply` as `token`: the status and body.
    fn apply_as(&self, token: &str, change: &Value) -> (StatusCode, Value) {
        let res = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .bearer_auth(token)
            .json(change)
            .send()
            .unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }
}

/// The round script both chat forms answer from: a search, then an answer.
fn push_search_then_answer(fake: &FakeAnthropic) {
    fake.push(AnthropicMessage::new().tool_use(
        "toolu_1",
        "search_notes",
        json!({ "text": "roof", "limit": 5 }),
    ));
    fake.push(AnthropicMessage::text("You chose slate."));
}

#[test]
fn a_turn_started_with_post_turns_streams_the_same_events_as_post_chat() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    api.memo("Roof decision", "We chose slate tiles for the roof.");
    let sent = json!([user("What did I decide about the roof?")]);

    push_search_then_answer(&fake);
    let posted = api.chat(sent.clone());

    push_search_then_answer(&fake);
    let reader = api.token_of("reader", "read");
    let (status, v) = api.start_turn(&reader, json!({ "messages": sent }));
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let turn = v["turn"].as_str().unwrap();
    let res = api.turn_events(&reader, turn);
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let streamed = read_events(res, |_| false);

    assert_eq!(shape(&streamed), shape(&posted), "{streamed:?}");
    assert_eq!(
        shape(&streamed),
        ["tool", "tool_result", "text", "done"],
        "{streamed:?}"
    );
    assert_eq!(text_of(&streamed), text_of(&posted));
    assert_eq!(
        last(&streamed, "tool_result")["summary"],
        last(&posted, "tool_result")["summary"]
    );
    assert_eq!(
        last(&streamed, "done")["messages"],
        last(&posted, "done")["messages"]
    );
    // The turn is over and kept a while (agents G17): read again, it replays, and after `n` it
    // starts with the event after the `n`th.
    let again = read_events(api.turn_events(&reader, turn), |_| false);
    assert_eq!(shape(&again), shape(&streamed));
    let n = streamed
        .iter()
        .position(|(e, _)| e == "tool_result")
        .unwrap()
        + 1;
    let res = api
        .client
        .get(
            api.server
                .url(&format!("/api/ai/chat/turns/{turn}/events?after={n}")),
        )
        .bearer_auth(&reader)
        .send()
        .unwrap();
    let rest = read_events(res, |_| false);
    assert_eq!(rest, streamed[n..].to_vec());
}

#[test]
fn a_turn_is_read_only_by_its_caller_by_any_number_of_readers() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    fake.push(AnthropicMessage::text(&"slow ".repeat(40)).delay(Duration::from_millis(20)));
    let turn = api.turn(json!([user("Go")]));
    let other = api.token_of("other", "read");
    assert_eq!(
        api.turn_events(&other, &turn).status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(api.stop_turn(&other, &turn), StatusCode::NOT_FOUND);
    let first = api.turn_events(&api.token(), &turn);
    assert_eq!(first.status(), StatusCode::OK);
    let second = api.turn_events(&api.token(), &turn);
    assert_eq!(second.status(), StatusCode::OK);
    let events = read_events(first, |_| false);
    assert_eq!(events.last().unwrap().0, "done", "{events:?}");
    assert_eq!(shape(&read_events(second, |_| false)), shape(&events));
}

#[test]
fn a_turn_nobody_reads_for_30_seconds_stops_and_asks_the_provider_no_more() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    // Round one's answer is longer than the turn holds unread, so the turn waits for its reader.
    fake.push(AnthropicMessage::text(&"slow ".repeat(200)).tool_use(
        "toolu_1",
        "search_notes",
        json!({ "text": "x", "limit": 1 }),
    ));
    fake.push(AnthropicMessage::text("never"));
    let turn = api.turn(json!([user("Go")]));
    std::thread::sleep(Duration::from_secs(33));
    assert_eq!(
        api.turn_events(&api.token(), &turn).status(),
        StatusCode::NOT_FOUND
    );
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(fake.message_requests().len(), 1);
}

#[test]
fn delete_stops_a_turn_while_it_is_read() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    fake.push(
        AnthropicMessage::text(&"slow ".repeat(60))
            .tool_use(
                "toolu_1",
                "search_notes",
                json!({ "text": "x", "limit": 1 }),
            )
            .delay(Duration::from_millis(20)),
    );
    fake.push(AnthropicMessage::text("never"));
    let turn = api.turn(json!([user("Go")]));
    let res = api.turn_events(&api.token(), &turn);
    let started = std::time::Instant::now();
    let mut stopped = false;
    let events = read_events(res, |(n, _)| {
        if n == "text" && !stopped {
            stopped = true;
            assert_eq!(api.stop_turn(&api.token(), &turn), StatusCode::NO_CONTENT);
        }
        false
    });
    // The stream ended soon after the stop, well before round one would have.
    assert!(started.elapsed() < Duration::from_secs(4), "{events:?}");
    assert!(!events.iter().any(|(n, _)| n == "done"), "{events:?}");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(fake.message_requests().len(), 1);
    assert_eq!(api.stop_turn(&api.token(), &turn), StatusCode::NOT_FOUND);
}

#[test]
fn a_fifth_running_turn_for_one_caller_is_refused() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    for _ in 0..6 {
        fake.push(AnthropicMessage::text(&"slow ".repeat(200)));
    }
    let turns: Vec<String> = (0..4).map(|_| api.turn(json!([user("Go")]))).collect();
    let (status, v) = api.start_turn(&api.token(), json!({ "messages": [user("Go")] }));
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
    assert_eq!(v["error"], "too_many_turns");
    // Another caller has turns of its own.
    let other = api.token_of("other", "read");
    let (status, _) = api.start_turn(&other, json!({ "messages": [user("Go")] }));
    assert_eq!(status, StatusCode::CREATED);
    // A stopped turn makes room.
    assert_eq!(
        api.stop_turn(&api.token(), &turns[0]),
        StatusCode::NO_CONTENT
    );
    let (status, v) = api.start_turn(&api.token(), json!({ "messages": [user("Go")] }));
    assert_eq!(status, StatusCode::CREATED, "{v}");
}

#[test]
fn get_providers_answers_a_read_token_with_no_key_token_or_account() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let plan = FakeOpenAi::start();
    api.chatgpt(&plan);
    api.secret("ai_anthropic_api_key", fake.api_key());
    api.secret(
        "ai_settings",
        &json!({
            "default_provider": "anthropic",
            "models": { "anthropic": MODEL, "chatgpt": PLAN_MODEL },
            "endpoints": { "anthropic": fake.base_url(), "chatgpt": plan.base_url() },
        })
        .to_string(),
    );
    let reader = api.token_of("reader", "read");
    let res = api
        .client
        .get(api.server.url("/api/ai/providers"))
        .bearer_auth(&reader)
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["default_provider"], "anthropic", "{v}");
    assert_eq!(
        v["providers"]["anthropic"],
        json!({ "configured": true, "model": MODEL })
    );
    assert_eq!(
        v["providers"]["chatgpt"],
        json!({ "configured": true, "model": PLAN_MODEL })
    );
    assert_eq!(v["providers"]["openai"]["configured"], false);
    assert_eq!(
        v["confirm"],
        json!({ "notes": true, "archive": true, "delete": true, "types": true, "downloads": true, "taxonomies": true, "agents": true })
    );
    for secret in [
        fake.api_key(),
        "plan-access-token",
        "plan-refresh-token",
        "me@example.com",
        "oaiapp_test",
        "account",
        "system_prompt",
        // No API URL either (ai D24).
        "url",
        fake.base_url().as_str(),
        "127.0.0.1",
    ] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    // Settings > AI itself stays admin-only.
    let res = api
        .client
        .get(api.server.url("/api/integrations/ai"))
        .bearer_auth(&reader)
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[test]
fn apply_writes_each_note_proposal_at_its_base_version_as_the_caller() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let agent = api.token_of("agent", "write");
    let (id, version) = api.memo("Groceries", "Milk");

    // An edit, as the chat sent it.
    let events = propose_one(
        &api,
        &fake,
        "propose_edit",
        json!({ "id": id, "base_version": version, "note": memo_doc("Groceries", "Milk and eggs").to_string() }),
    );
    let edit = last(&events, "proposal").clone();
    let (status, done) = api.apply_as(&agent, &edit);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "update");
    assert_eq!(done["note_id"], id.as_str());
    let note = api.note(&id);
    assert_eq!(note["version"], done["version"]);
    assert!(note.to_string().contains("Milk and eggs"));
    let history = api.get(&format!("/api/notes/{id}/history"));
    assert!(history.to_string().contains("agent"), "{history}");

    // The same edit again is on a stale version: a conflict, and nothing is written.
    let (status, err) = api.apply_as(&agent, &edit);
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["error"], "conflict");
    assert_eq!(api.note(&id)["version"], done["version"]);

    // A create.
    let events = propose_one(
        &api,
        &fake,
        "propose_create",
        json!({ "type": "memo", "note": memo_doc("Reading list", "Dune").to_string() }),
    );
    let (status, created) = api.apply_as(&agent, last(&events, "proposal"));
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["op"], "create");
    let new_id = created["note_id"].as_str().unwrap();
    assert!(api.note(new_id).to_string().contains("Dune"));

    // An archive, then an unarchive, each at the version it was proposed on.
    let events = propose_one(
        &api,
        &fake,
        "propose_archive",
        json!({ "id": new_id, "archived": true }),
    );
    let archive = last(&events, "proposal").clone();
    let (status, archived) = api.apply_as(&agent, &archive);
    assert_eq!(status, StatusCode::OK, "{archived}");
    assert_eq!(archived["op"], "archive");
    assert_eq!(api.note(new_id)["note"]["fields"]["archived"], true);
    let (status, _) = api.apply_as(&agent, &archive);
    assert_eq!(status, StatusCode::CONFLICT);
    let events = propose_one(
        &api,
        &fake,
        "propose_archive",
        json!({ "id": new_id, "archived": false }),
    );
    let (status, unarchived) = api.apply_as(&agent, last(&events, "proposal"));
    assert_eq!(status, StatusCode::OK, "{unarchived}");
    assert_eq!(unarchived["op"], "unarchive");

    // A delete on a stale version writes nothing; on the current one the note goes to the trash.
    let events = propose_one(&api, &fake, "propose_delete", json!({ "id": id }));
    let delete = last(&events, "proposal").clone();
    let mut stale = delete.clone();
    stale["base_version"] = json!(version);
    let (status, _) = api.apply_as(&agent, &stale);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(api.memos().len(), 2);
    let (status, deleted) = api.apply_as(&agent, &delete);
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["op"], "delete");
    let trash = api.get("/api/trash");
    assert_eq!(trash["notes"][0]["id"], id.as_str(), "{trash}");

    // A read token may chat but not apply.
    let reader = api.token_of("reader", "read");
    let (status, _) = api.apply_as(&reader, &archive);
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn apply_needs_admin_for_a_type_change_and_refuses_a_stale_one() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let changed = MEMO.replace("tab \"Main\"", "tab \"Details\"");
    let events = propose_one(
        &api,
        &fake,
        "propose_type_change",
        json!({ "name": "memo", "wcl": changed }),
    );
    let proposal = last(&events, "type_proposal").clone();

    let writer = api.token_of("writer", "write");
    let (status, err) = api.apply_as(&writer, &proposal);
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["error"], "needs_admin");
    assert_eq!(err["message"], "Apply type changes in the web app.");
    assert!(api.type_wcl("memo").unwrap().contains("tab \"Main\""));

    let (status, done) = api.apply_as(&api.token(), &proposal);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done, json!({ "op": "type_change", "type_name": "memo" }));
    assert!(api.type_wcl("memo").unwrap().contains("tab \"Details\""));

    // Its before is no longer the type file.
    let (status, err) = api.apply_as(&api.token(), &proposal);
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["error"], "stale");

    // Something that is not a change is a 400.
    let (status, _) = api.apply_as(&api.token(), &json!({ "op": "purge", "note_id": "x" }));
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[test]
fn find_assets_matches_file_names_newest_first_and_keeps_to_its_limit() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake);
    let mut ids = Vec::new();
    for name in [
        "Elden-Ring-Cover.jpg",
        "hades-ii.png",
        "notes.md",
        "elden-ring-map.png",
    ] {
        ids.push(api.upload(name, b"x".to_vec()));
        // A new millisecond, so the ids sort in upload order.
        std::thread::sleep(Duration::from_millis(3));
    }
    fake.push(
        AnthropicMessage::new()
            .tool_use(
                "toolu_match",
                "find_assets",
                json!({ "text": "ELDEN", "limit": 20 }),
            )
            .tool_use(
                "toolu_all",
                "find_assets",
                json!({ "text": "", "limit": 20 }),
            )
            .tool_use(
                "toolu_two",
                "find_assets",
                json!({ "text": " ", "limit": 2 }),
            )
            .tool_use(
                "toolu_none",
                "find_assets",
                json!({ "text": "zzz", "limit": 0 }),
            ),
    );
    fake.push(AnthropicMessage::text("Found them."));
    let events = api.chat(json!([user("Find the Elden Ring pictures.")]));
    assert_eq!(names(&events).last(), Some(&"done"), "{events:?}");
    let body = fake.message_requests()[1].json();
    let ids_of = |call: &str| -> Vec<String> {
        let got = result_text(&anthropic_result(&body, call));
        got["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_owned())
            .collect()
    };

    // A match ignores case, newest first, each with its metadata.
    assert_eq!(ids_of("toolu_match"), [ids[3].clone(), ids[0].clone()]);
    let got = result_text(&anthropic_result(&body, "toolu_match"));
    let first = &got["assets"][0];
    assert_eq!(first["filename"], "elden-ring-map.png");
    assert_eq!(first["media_type"], "image/png");
    assert_eq!(first["size"], 1);
    assert!(first["created"].as_str().unwrap().ends_with('Z'), "{first}");
    assert_eq!(got["total"], 2);

    // Empty text lists the newest; the limit keeps to it, and the total says how many matched.
    let mut newest = ids.clone();
    newest.reverse();
    assert_eq!(ids_of("toolu_all"), newest);
    assert_eq!(ids_of("toolu_two"), newest[..2].to_vec());
    let got = result_text(&anthropic_result(&body, "toolu_two"));
    assert_eq!(got["total"], 4);
    assert_eq!(ids_of("toolu_none"), Vec::<String>::new());

    // The step's summary names the assets, never their bytes.
    let summary = events
        .iter()
        .find(|(n, d)| n == "tool_result" && d["id"] == "toolu_match")
        .map(|(_, d)| d)
        .unwrap();
    assert_eq!(summary["is_error"], false);
    assert_eq!(
        summary["summary"]["assets"][1]["filename"],
        "Elden-Ring-Cover.jpg"
    );
}
