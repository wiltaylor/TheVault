//! S1 with FakeBrave and FakeAnthropic: Settings > Integrations > Brave Search's routes, and the
//! chat's `web_search` tool, offered only with a key (ai D26).

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, BraveReply, BraveResult, FakeAnthropic, FakeBrave, TestServer};

const MODEL: &str = "claude-opus-5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Api {
            server: TestServer::start(),
            client,
        }
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

    /// Point the server at `fake`, without storing its key.
    fn brave_endpoint(&self, fake: &FakeBrave) {
        self.secret(
            "brave_settings",
            &json!({ "endpoint": fake.base_url() }).to_string(),
        );
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
        let text = res.text().unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    fn put_key(&self, key: &str) -> Value {
        let (status, body) = self.send(
            reqwest::Method::PUT,
            "/api/integrations/brave/key",
            Some(json!({ "key": key })),
        );
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn test(&self) -> Value {
        let (status, body) = self.send(reqwest::Method::POST, "/api/integrations/brave/test", None);
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// One chat turn, read to the end of its stream.
    fn chat(&self, question: &str) -> Vec<(String, Value)> {
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(self.token())
            .json(&json!({ "messages": [
                { "role": "user", "content": [{ "type": "text", "text": question }] }
            ] }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
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

fn event<'a>(events: &'a [(String, Value)], name: &str) -> &'a Value {
    &events
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
        .1
}

/// The names of the tools the model was offered in request `n`.
fn offered(fake: &FakeAnthropic, n: usize) -> Vec<String> {
    fake.message_requests()[n].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The tool result the model got for call `call`, its text parsed as JSON, and its error flag.
fn result_for(fake: &FakeAnthropic, call: &str) -> (Value, bool) {
    for r in fake.message_requests() {
        for m in r.json()["messages"].as_array().unwrap() {
            for c in m["content"].as_array().unwrap() {
                if c["type"] == "tool_result" && c["tool_use_id"] == call {
                    let text = match &c["content"] {
                        Value::String(s) => s.clone(),
                        Value::Array(b) => b[0]["text"].as_str().unwrap().to_owned(),
                        other => panic!("{other}"),
                    };
                    return (
                        serde_json::from_str(&text).unwrap(),
                        c["is_error"] == json!(true),
                    );
                }
            }
        }
    }
    panic!("no result for {call}");
}

#[test]
fn the_key_stores_never_comes_back_and_deletes() {
    let api = Api::start();
    let (_, status) = api.send(reqwest::Method::GET, "/api/integrations/brave", None);
    assert_eq!(status, json!({ "configured": false }));

    let stored = api.put_key("  BSA-secret-key  ");
    assert_eq!(stored, json!({ "configured": true }));
    assert_eq!(
        api.server.secret("brave_api_key").as_deref(),
        Some("BSA-secret-key")
    );
    let (_, status) = api.send(reqwest::Method::GET, "/api/integrations/brave", None);
    assert_eq!(status, json!({ "configured": true }));
    let (_, secret) = api.send(reqwest::Method::GET, "/api/secrets/brave_api_key", None);
    assert!(!secret.to_string().contains("BSA-secret-key"), "{secret}");

    assert_eq!(api.put_key(""), json!({ "configured": false }));
    assert_eq!(api.server.secret("brave_api_key"), None);
    // Removing a key that is not there is not an error.
    assert_eq!(api.put_key(""), json!({ "configured": false }));
}

#[test]
fn the_routes_need_an_admin_token() {
    let api = Api::start();
    let res = api
        .client
        .get(api.server.url("/api/integrations/brave"))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn test_reports_the_result_count_or_braves_error() {
    let api = Api::start();
    let fake = FakeBrave::start();
    api.brave_endpoint(&fake);

    let none = api.test();
    assert_eq!(none["ok"], false);
    assert_eq!(none["error"]["kind"], "not_configured");
    assert!(fake.requests().is_empty());

    api.put_key(fake.api_key());
    let ok = api.test();
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["count"], 2);
    assert_eq!(ok["error"], Value::Null);
    let sent = &fake.search_requests()[0];
    assert_eq!(sent.query("q").as_deref(), Some("TheVault"));
    assert_eq!(sent.header("x-subscription-token"), Some(fake.api_key()));
    assert_eq!(sent.header("accept"), Some("application/json"));

    api.put_key("BSA-wrong");
    let refused = api.test();
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["kind"], "auth");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("The provided subscription token is invalid."),
        "{refused}"
    );
    assert!(!refused.to_string().contains("BSA-wrong"));
}

#[test]
fn with_no_key_the_chat_offers_no_web_search() {
    let api = Api::start();
    let model = FakeAnthropic::start();
    api.anthropic(&model);
    model.push(AnthropicMessage::text("Hello."));
    api.chat("hi");
    let tools = offered(&model, 0);
    assert!(tools.contains(&"search_notes".to_owned()));
    assert!(!tools.contains(&"web_search".to_owned()), "{tools:?}");
}

#[test]
fn with_a_key_web_search_returns_stripped_results_and_the_query_stays_out_of_the_trail() {
    let api = Api::start();
    let model = FakeAnthropic::start();
    api.anthropic(&model);
    let brave = FakeBrave::start();
    api.brave_endpoint(&brave);
    api.put_key(brave.api_key());
    brave.push(BraveReply::results(vec![
        BraveResult::new(
            "Slate <strong>roof</strong> tiles &amp; costs",
            "https://example.com/slate",
            "How much a <strong>slate</strong> roof costs &#x27;today&#x27;.",
        )
        .age("2 days ago"),
        BraveResult::new(
            "Roofing guide",
            "https://example.com/guide",
            "All about roofs.",
        ),
    ]));
    let query = "zanzibar-slate-roof-prices";
    model.push(AnthropicMessage::new().tool_use(
        "toolu_w",
        "web_search",
        json!({ "query": query, "count": 3 }),
    ));
    model.push(AnthropicMessage::text("Slate costs a lot."));

    let events = api.chat("what does a slate roof cost?");
    assert!(offered(&model, 0).contains(&"web_search".to_owned()));

    let sent = &brave.search_requests()[0];
    assert_eq!(sent.query("q").as_deref(), Some(query));
    assert_eq!(sent.query("count").as_deref(), Some("3"));

    let (result, is_error) = result_for(&model, "toolu_w");
    assert!(!is_error);
    assert_eq!(
        result,
        json!([
            {
                "title": "Slate roof tiles & costs",
                "url": "https://example.com/slate",
                "description": "How much a slate roof costs 'today'.",
                "age": "2 days ago",
            },
            { "title": "Roofing guide", "url": "https://example.com/guide", "description": "All about roofs." },
        ])
    );

    let tool = event(&events, "tool");
    assert_eq!(tool["name"], "web_search");
    let summary = &event(&events, "tool_result")["summary"];
    assert_eq!(
        summary["links"],
        json!([
            { "title": "Slate roof tiles & costs", "url": "https://example.com/slate" },
            { "title": "Roofing guide", "url": "https://example.com/guide" },
        ])
    );
    assert_eq!(event(&events, "done")["stop"], "end_turn");

    let lines = api.server.stderr_lines();
    let trail: Vec<&String> = lines.iter().filter(|l| l.starts_with("brave: ")).collect();
    assert_eq!(trail.len(), 1, "{lines:?}");
    assert!(
        trail[0].starts_with("brave: count=3 results=2 outcome=ok ms="),
        "{}",
        trail[0]
    );
    assert!(
        lines.iter().all(|l| !l.contains(query)),
        "the query is in the server log: {lines:?}"
    );
}

#[test]
fn rate_limits_and_refused_keys_are_error_results_with_their_kind() {
    let api = Api::start();
    let model = FakeAnthropic::start();
    api.anthropic(&model);
    let brave = FakeBrave::start();
    api.brave_endpoint(&brave);
    api.put_key(brave.api_key());
    brave.push(BraveReply::rate_limited());
    brave.push(BraveReply::unauthorized());
    model.push(
        AnthropicMessage::new()
            .tool_use(
                "toolu_a",
                "web_search",
                json!({ "query": "one", "count": 5 }),
            )
            .tool_use(
                "toolu_b",
                "web_search",
                json!({ "query": "two", "count": 5 }),
            ),
    );
    model.push(AnthropicMessage::text("Brave is not answering."));

    let events = api.chat("search twice");

    let (limited, is_error) = result_for(&model, "toolu_a");
    assert!(is_error);
    assert_eq!(limited["kind"], "rate_limit");
    assert!(
        limited["error"]
            .as_str()
            .unwrap()
            .contains("Request rate limit exceeded for plan."),
        "{limited}"
    );
    let (refused, is_error) = result_for(&model, "toolu_b");
    assert!(is_error);
    assert_eq!(refused["kind"], "auth");

    let results: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "tool_result")
        .map(|(_, d)| d)
        .collect();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r["is_error"] == true));
    assert_eq!(results[0]["summary"]["kind"], "rate_limit");

    let lines = api.server.stderr_lines();
    let trail: Vec<&String> = lines.iter().filter(|l| l.starts_with("brave: ")).collect();
    assert_eq!(trail.len(), 2, "{lines:?}");
    assert!(
        trail[0].contains("outcome=error:rate_limit"),
        "{}",
        trail[0]
    );
    assert!(trail[1].contains("outcome=error:auth"), "{}", trail[1]);
}
