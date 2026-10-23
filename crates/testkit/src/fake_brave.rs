//! A fake of the Brave Search web search API, for the chat's `web_search` tool and the Brave
//! card's Test (ai D26).
//!
//! ```no_run
//! use testkit::{BraveReply, BraveResult, FakeBrave};
//!
//! let fake = FakeBrave::start();
//! // brave_settings = {"endpoint": fake.base_url()}; brave_api_key = fake.api_key().
//! fake.push(BraveReply::results(vec![BraveResult::new("A <strong>title</strong>", "https://example.com/a", "Some text")]));
//! fake.push(BraveReply::rate_limited());
//! fake.push(BraveReply::unauthorized());
//! // ... after the calls:
//! let sent = fake.search_requests(); // each request's query, count and headers
//! ```
//!
//! `GET /res/v1/web/search` answers the next queued reply, or [`FakeBrave::default_results`]
//! when none is queued, cut to the request's `count`. It checks the `X-Subscription-Token`
//! (401 `SUBSCRIPTION_TOKEN_INVALID` otherwise) and that `q` is present (422 otherwise), as Brave
//! does, and answers errors in Brave's `ErrorResponse` shape. It binds `127.0.0.1` on a free
//! port and records every request.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::fake_http::{Answer, Listener, listen, lock};
use crate::standin::{Reply, Request};

/// The key the fake accepts.
pub const DEFAULT_BRAVE_KEY: &str = "BSA-test-0123456789";

/// The search path.
pub const BRAVE_SEARCH_PATH: &str = "/res/v1/web/search";

/// One web result.
#[derive(Debug, Clone)]
pub struct BraveResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub age: Option<String>,
}

impl BraveResult {
    pub fn new(title: &str, url: &str, description: &str) -> BraveResult {
        BraveResult {
            title: title.to_owned(),
            url: url.to_owned(),
            description: description.to_owned(),
            age: None,
        }
    }

    pub fn age(mut self, age: &str) -> BraveResult {
        self.age = Some(age.to_owned());
        self
    }

    fn to_json(&self) -> Value {
        let mut o = json!({
            "type": "search_result",
            "title": self.title,
            "url": self.url,
            "description": self.description,
            "is_source_local": false,
            "language": "en",
            "profile": { "name": "Example", "url": self.url },
        });
        if let Some(age) = &self.age {
            o["age"] = json!(age);
            o["page_age"] = json!("2026-10-20T00:00:00");
        }
        o
    }
}

/// One answer to a search.
#[derive(Debug, Clone)]
pub enum BraveReply {
    Results(Vec<BraveResult>),
    Http(Reply),
}

impl BraveReply {
    pub fn results(results: Vec<BraveResult>) -> BraveReply {
        BraveReply::Results(results)
    }

    /// 429 `RATE_LIMITED`, as Brave answers over the plan's rate.
    pub fn rate_limited() -> BraveReply {
        BraveReply::error(429, "RATE_LIMITED", "Request rate limit exceeded for plan.")
    }

    /// 401 `SUBSCRIPTION_TOKEN_INVALID`.
    pub fn unauthorized() -> BraveReply {
        BraveReply::error(
            401,
            "SUBSCRIPTION_TOKEN_INVALID",
            "The provided subscription token is invalid.",
        )
    }

    /// Brave's `ErrorResponse` with `status`, `code` and `detail`.
    pub fn error(status: u16, code: &str, detail: &str) -> BraveReply {
        BraveReply::Http(Reply::json(status, &error_body(status, code, detail)))
    }
}

fn error_body(status: u16, code: &str, detail: &str) -> Value {
    json!({
        "type": "ErrorResponse",
        "error": { "id": "fake-error", "status": status, "code": code, "detail": detail, "meta": {} },
        "time": 1_761_000_000,
    })
}

struct Shared {
    key: String,
    requests: Mutex<Vec<Request>>,
    replies: Mutex<VecDeque<BraveReply>>,
    default_results: Mutex<Vec<BraveResult>>,
}

/// A running fake Brave Search API. Dropping it stops the server.
pub struct FakeBrave {
    listener: Listener,
    shared: Arc<Shared>,
}

impl FakeBrave {
    /// Start with [`DEFAULT_BRAVE_KEY`] and two default results.
    pub fn start() -> FakeBrave {
        let shared = Arc::new(Shared {
            key: DEFAULT_BRAVE_KEY.to_owned(),
            requests: Mutex::default(),
            replies: Mutex::default(),
            default_results: Mutex::new(vec![
                BraveResult::new(
                    "<strong>TheVault</strong> on GitHub",
                    "https://github.com/wiltaylor/TheVault",
                    "A self-hosted <strong>note</strong> app.",
                ),
                BraveResult::new(
                    "TheVault docs",
                    "https://example.com/thevault",
                    "How to use it.",
                )
                .age("3 days ago"),
            ]),
        });
        let s = shared.clone();
        let listener = listen("the fake Brave Search API", move |r| handle(&s, r));
        FakeBrave { listener, shared }
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener.addr
    }

    /// `http://127.0.0.1:<port>`: the value for `brave_settings.endpoint`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.listener.addr)
    }

    /// The key it accepts, for `brave_api_key`.
    pub fn api_key(&self) -> &str {
        &self.shared.key
    }

    /// Queue the answer to a later search, in order.
    pub fn push(&self, reply: BraveReply) {
        lock(&self.shared.replies).push_back(reply);
    }

    /// Answer searches with no queued reply with `results`.
    pub fn set_default_results(&self, results: Vec<BraveResult>) {
        *lock(&self.shared.default_results) = results;
    }

    /// The results a search with no queued reply gets.
    pub fn default_results(&self) -> Vec<BraveResult> {
        lock(&self.shared.default_results).clone()
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }

    /// The `GET /res/v1/web/search` requests received so far.
    pub fn search_requests(&self) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == "GET" && r.route() == BRAVE_SEARCH_PATH)
            .collect()
    }
}

fn handle(shared: &Shared, request: &Request) -> Answer {
    lock(&shared.requests).push(request.clone());
    let err = |status: u16, code: &str, detail: &str| {
        Answer::Whole(Reply::json(status, &error_body(status, code, detail)))
    };
    if (request.method.as_str(), request.route()) != ("GET", BRAVE_SEARCH_PATH) {
        return err(404, "NOT_FOUND", "Not found");
    }
    if request.header("x-subscription-token") != Some(shared.key.as_str()) {
        return err(
            401,
            "SUBSCRIPTION_TOKEN_INVALID",
            "The provided subscription token is invalid.",
        );
    }
    let Some(q) = request.query("q").filter(|q| !q.is_empty()) else {
        return err(422, "VALIDATION", "Unable to validate request parameter(s)");
    };
    let count = request
        .query("count")
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 20);
    let reply = lock(&shared.replies)
        .pop_front()
        .unwrap_or_else(|| BraveReply::Results(lock(&shared.default_results).clone()));
    match reply {
        BraveReply::Http(r) => Answer::Whole(r),
        BraveReply::Results(rs) => {
            let results: Vec<Value> = rs.iter().take(count).map(BraveResult::to_json).collect();
            Answer::Whole(Reply::json(
                200,
                &json!({
                    "type": "search",
                    "query": { "original": q, "more_results_available": false },
                    "mixed": { "type": "mixed", "main": [], "top": [], "side": [] },
                    "web": { "type": "search", "results": results, "family_friendly": true },
                }),
            ))
        }
    }
}
