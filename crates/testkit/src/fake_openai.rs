//! A fake of OpenAI's Responses and Models APIs, answering both an API key and a ChatGPT plan
//! access token, for the `ai` crate and the routes and scripts built on it.
//!
//! ```no_run
//! use serde_json::json;
//! use testkit::{FakeOpenAi, OpenAiReply, OpenAiResponse};
//!
//! let fake = FakeOpenAi::start();
//! // ai_settings.endpoints.openai = fake.base_url(); the API key is fake.api_key().
//! fake.push(OpenAiResponse::text("Hello there"));
//! fake.push(OpenAiResponse::new().function_call("call_1", "get_note", json!({"id": "01J"})));
//! fake.push(OpenAiResponse::new().refusal("I can't help with that."));
//! fake.push(OpenAiResponse::text("partial").failed("rate_limit_exceeded", "Slow down"));
//! fake.push(OpenAiReply::error(429, "subscription_sharing_usage_limit_exceeded", "Limit"));
//! let sent = fake.response_requests();
//! ```
//!
//! A request whose bearer token is the API key is an API-key request; any other bearer token is
//! a ChatGPT plan token. `POST /v1/responses` answers the next queued reply, or
//! [`OpenAiResponse::text`] `"OK"` when none is queued: streamed (`response.created`, then per
//! output item `response.output_item.added`, its `response.output_text.delta`s,
//! `response.refusal.delta`s or `response.function_call_arguments.delta`s and
//! `response.output_item.done`, then `response.completed`, `response.incomplete` or
//! `response.failed`) when the body says `stream: true`, whole otherwise.
//!
//! A plan request is checked against the plan's rules: it must send `store: false` and
//! `stream: true`, and none of `max_output_tokens`, `previous_response_id`, `metadata`, `user`,
//! `truncation`, `background`, `conversation`, `max_tool_calls`, `temperature` or `top_p`, nor a
//! `role: "system"` item; a breach answers 400 `subscription_sharing_unsupported_capability`
//! naming the parameter. `GET /v1/models` answers `{object, data: [{id}]}` to the API key and
//! `{models: [{slug, display_name, visibility}]}` to a plan token.
//!
//! It binds `127.0.0.1` on a free port. Every request is recorded, whatever it was answered.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::fake_http::{Answer, Listener, Streamed, frame, listen, lock, pieces};
use crate::standin::{Reply, Request};

/// The API key the fake accepts unless [`FakeOpenAiBuilder::api_key`] says otherwise.
pub const DEFAULT_OPENAI_KEY: &str = "sk-test-openai-0123456789";

/// The fields a ChatGPT plan request must not send.
pub const PLAN_FORBIDDEN: [&str; 10] = [
    "max_output_tokens",
    "previous_response_id",
    "metadata",
    "user",
    "truncation",
    "background",
    "conversation",
    "max_tool_calls",
    "temperature",
    "top_p",
];

/// One answer to `POST /v1/responses`.
#[derive(Debug, Clone)]
pub enum OpenAiReply {
    Response(OpenAiResponse),
    Http(Reply),
}

impl OpenAiReply {
    /// OpenAI's error body `{error: {message, type, code, param}}` with `status`.
    pub fn error(status: u16, code: &str, message: &str) -> OpenAiReply {
        OpenAiReply::Http(Reply::json(
            status,
            &json!({ "error": { "message": message, "type": "invalid_request_error", "code": code, "param": null } }),
        ))
    }

    /// An admission failure before the stream opens: `{detail}` with `status`.
    pub fn detail(status: u16, detail: &str) -> OpenAiReply {
        OpenAiReply::Http(Reply::json(status, &json!({ "detail": detail })))
    }

    /// 429 `rate_limit_exceeded` with `retry-after: <seconds>`.
    pub fn rate_limited(retry_after_secs: u64) -> OpenAiReply {
        match OpenAiReply::error(429, "rate_limit_exceeded", "Rate limit reached") {
            OpenAiReply::Http(r) => {
                OpenAiReply::Http(r.header("retry-after", &retry_after_secs.to_string()))
            }
            other => other,
        }
    }
}

impl From<OpenAiResponse> for OpenAiReply {
    fn from(r: OpenAiResponse) -> Self {
        OpenAiReply::Response(r)
    }
}

impl From<Reply> for OpenAiReply {
    fn from(r: Reply) -> Self {
        OpenAiReply::Http(r)
    }
}

/// How a response ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Completed,
    /// `response.incomplete` with `incomplete_details.reason`.
    Incomplete(String),
    /// `response.failed` with `error.code` and `error.message`.
    Failed(String, String),
}

/// A response the fake model answers, built item by item.
#[derive(Debug, Clone)]
pub struct OpenAiResponse {
    /// The output items, without their ids, which the fake adds.
    pub output: Vec<Value>,
    pub ending: Ending,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The size of each streamed delta, in characters.
    pub chunk: usize,
    /// Close the stream after this many frames.
    pub cut_after: Option<usize>,
    pub delay: Duration,
}

impl Default for OpenAiResponse {
    fn default() -> Self {
        OpenAiResponse {
            output: Vec::new(),
            ending: Ending::Completed,
            input_tokens: 10,
            output_tokens: 5,
            chunk: 4,
            cut_after: None,
            delay: Duration::ZERO,
        }
    }
}

impl OpenAiResponse {
    pub fn new() -> Self {
        Self::default()
    }

    /// A response of one message of `text`.
    pub fn text(text: &str) -> Self {
        Self::new().with_text(text)
    }

    /// Add a message item of `text`.
    pub fn with_text(mut self, text: &str) -> Self {
        self.output.push(json!({
            "type": "message", "status": "completed", "role": "assistant",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }],
        }));
        self
    }

    /// Add a reasoning item with its encrypted content.
    pub fn reasoning(mut self, summary: &str, encrypted_content: &str) -> Self {
        self.output.push(json!({
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": summary }],
            "encrypted_content": encrypted_content,
        }));
        self
    }

    /// Add a function call.
    pub fn function_call(self, call_id: &str, name: &str, arguments: Value) -> Self {
        self.function_call_raw(call_id, name, &arguments.to_string())
    }

    /// Add a function call whose arguments are `raw`, which need not be JSON.
    pub fn function_call_raw(mut self, call_id: &str, name: &str, raw: &str) -> Self {
        self.output.push(json!({
            "type": "function_call", "status": "completed", "call_id": call_id, "name": name,
            "arguments": raw,
        }));
        self
    }

    /// Add a message item whose content is a refusal.
    pub fn refusal(mut self, text: &str) -> Self {
        self.output.push(json!({
            "type": "message", "status": "completed", "role": "assistant",
            "content": [{ "type": "refusal", "refusal": text }],
        }));
        self
    }

    /// End `response.incomplete` with `reason`, such as `max_output_tokens`.
    pub fn incomplete(mut self, reason: &str) -> Self {
        self.ending = Ending::Incomplete(reason.to_owned());
        self
    }

    /// End `response.failed` with `code` and `message`.
    pub fn failed(mut self, code: &str, message: &str) -> Self {
        self.ending = Ending::Failed(code.to_owned(), message.to_owned());
        self
    }

    pub fn usage(mut self, input: u64, output: u64) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }

    pub fn chunk(mut self, n: usize) -> Self {
        self.chunk = n.max(1);
        self
    }

    /// Break the stream off after `frames` frames.
    pub fn cut_after(mut self, frames: usize) -> Self {
        self.cut_after = Some(frames);
        self
    }

    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// The output items with ids.
    fn items(&self, n: u64) -> Vec<Value> {
        self.output
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mut item = item.clone();
                let prefix = match item["type"].as_str() {
                    Some("reasoning") => "rs",
                    Some("function_call") => "fc",
                    _ => "msg",
                };
                item["id"] = json!(format!("{prefix}_fake_{n}_{i}"));
                item
            })
            .collect()
    }

    fn whole(&self, model: &str, n: u64) -> Value {
        let mut r = json!({
            "id": format!("resp_fake_{n}"), "object": "response", "model": model,
            "output": self.items(n),
            "usage": { "input_tokens": self.input_tokens, "output_tokens": self.output_tokens,
                       "total_tokens": self.input_tokens + self.output_tokens },
        });
        match &self.ending {
            Ending::Completed => r["status"] = json!("completed"),
            Ending::Incomplete(reason) => {
                r["status"] = json!("incomplete");
                r["incomplete_details"] = json!({ "reason": reason });
            }
            Ending::Failed(code, message) => {
                r["status"] = json!("failed");
                r["error"] = json!({ "code": code, "message": message });
                r["output"] = json!([]);
            }
        }
        r
    }

    fn frames(&self, model: &str, n: u64) -> Vec<String> {
        let mut seq = 0u64;
        let mut ev = |kind: &str, mut data: Value| {
            data["type"] = json!(kind);
            data["sequence_number"] = json!(seq);
            seq += 1;
            frame(kind, &data)
        };
        let id = format!("resp_fake_{n}");
        let mut f = vec![ev(
            "response.created",
            json!({ "response": { "id": id, "object": "response", "status": "in_progress", "model": model, "output": [] } }),
        )];
        let items = self.items(n);
        for (index, item) in items.iter().enumerate() {
            let item_id = item["id"].clone();
            let mut skeleton = item.clone();
            match item["type"].as_str() {
                Some("message") => skeleton["content"] = json!([]),
                Some("function_call") => skeleton["arguments"] = json!(""),
                _ => {}
            }
            skeleton["status"] = json!("in_progress");
            f.push(ev(
                "response.output_item.added",
                json!({ "output_index": index, "item": skeleton }),
            ));
            match item["type"].as_str() {
                Some("message") => {
                    for (ci, part) in item["content"].as_array().into_iter().flatten().enumerate() {
                        let (kind, text) = match part["type"].as_str() {
                            Some("refusal") => ("response.refusal.delta", &part["refusal"]),
                            _ => ("response.output_text.delta", &part["text"]),
                        };
                        for p in pieces(text.as_str().unwrap_or_default(), self.chunk) {
                            f.push(ev(kind, json!({ "item_id": item_id, "output_index": index, "content_index": ci, "delta": p })));
                        }
                    }
                }
                Some("function_call") => {
                    for p in pieces(item["arguments"].as_str().unwrap_or_default(), self.chunk) {
                        f.push(ev(
                            "response.function_call_arguments.delta",
                            json!({ "item_id": item_id, "output_index": index, "delta": p }),
                        ));
                    }
                }
                _ => {}
            }
            f.push(ev(
                "response.output_item.done",
                json!({ "output_index": index, "item": item }),
            ));
        }
        let whole = self.whole(model, n);
        let last = match &self.ending {
            Ending::Completed => "response.completed",
            Ending::Incomplete(_) => "response.incomplete",
            Ending::Failed(..) => "response.failed",
        };
        f.push(ev(last, json!({ "response": whole })));
        f
    }
}

/// How a [`FakeOpenAi`] starts.
#[derive(Debug, Clone)]
pub struct FakeOpenAiBuilder {
    api_key: String,
    models: Vec<String>,
    plan_models: Vec<(String, String, String)>,
}

impl Default for FakeOpenAiBuilder {
    fn default() -> Self {
        FakeOpenAiBuilder {
            api_key: DEFAULT_OPENAI_KEY.to_owned(),
            models: vec![
                "gpt-6".to_owned(),
                "gpt-6-mini".to_owned(),
                "text-embedding-4".to_owned(),
            ],
            plan_models: vec![
                (
                    "gpt-6.1-sol".to_owned(),
                    "GPT-6.1 Sol".to_owned(),
                    "list".to_owned(),
                ),
                (
                    "gpt-6-luna".to_owned(),
                    "GPT-6 Luna".to_owned(),
                    "list".to_owned(),
                ),
                (
                    "gpt-6-internal".to_owned(),
                    "Internal".to_owned(),
                    "hide".to_owned(),
                ),
            ],
        }
    }
}

impl FakeOpenAiBuilder {
    /// The bearer token that makes a request an API-key one.
    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = key.to_owned();
        self
    }

    /// The model ids `/v1/models` lists to the API key.
    pub fn models(mut self, ids: &[&str]) -> Self {
        self.models = ids.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// The models `/v1/models` lists to a plan token, `(slug, display_name, visibility)`.
    pub fn plan_models(mut self, models: &[(&str, &str, &str)]) -> Self {
        self.plan_models = models
            .iter()
            .map(|(s, d, v)| ((*s).to_owned(), (*d).to_owned(), (*v).to_owned()))
            .collect();
        self
    }

    pub fn start(self) -> FakeOpenAi {
        let shared = Arc::new(Shared {
            config: self,
            requests: Mutex::default(),
            replies: Mutex::default(),
            served: Mutex::new(0),
        });
        let s = shared.clone();
        let listener = listen("the fake OpenAI API", move |r| handle(&s, r));
        FakeOpenAi { listener, shared }
    }
}

struct Shared {
    config: FakeOpenAiBuilder,
    requests: Mutex<Vec<Request>>,
    replies: Mutex<VecDeque<OpenAiReply>>,
    served: Mutex<u64>,
}

/// A running fake OpenAI API. Dropping it stops the server.
pub struct FakeOpenAi {
    listener: Listener,
    shared: Arc<Shared>,
}

impl FakeOpenAi {
    pub fn builder() -> FakeOpenAiBuilder {
        FakeOpenAiBuilder::default()
    }

    pub fn start() -> FakeOpenAi {
        Self::builder().start()
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener.addr
    }

    /// `http://127.0.0.1:<port>`: the value for `ai_settings.endpoints.openai`, which the
    /// ChatGPT plan uses too.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.listener.addr)
    }

    /// The API key it accepts, for `ai_openai_api_key`.
    pub fn api_key(&self) -> &str {
        &self.shared.config.api_key
    }

    /// Queue the answer to a later `POST /v1/responses`, in order.
    pub fn push(&self, reply: impl Into<OpenAiReply>) {
        lock(&self.shared.replies).push_back(reply.into());
    }

    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }

    /// The `POST /v1/responses` requests received so far.
    pub fn response_requests(&self) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == "POST" && r.route() == "/v1/responses")
            .collect()
    }

    pub fn clear_requests(&self) {
        lock(&self.shared.requests).clear();
    }
}

fn error(status: u16, code: &str, message: &str, param: Option<&str>) -> Answer {
    Answer::Whole(Reply::json(
        status,
        &json!({ "error": { "message": message, "type": "invalid_request_error", "code": code, "param": param } }),
    ))
}

fn handle(shared: &Shared, request: &Request) -> Answer {
    lock(&shared.requests).push(request.clone());
    let Some(token) = request
        .header("authorization")
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return error(
            401,
            "invalid_api_key",
            "You didn't provide an API key.",
            None,
        );
    };
    let plan = token != shared.config.api_key;
    match (request.method.as_str(), request.route()) {
        ("POST", "/v1/responses") => responses(shared, request, plan),
        ("GET", "/v1/models") => models(shared, plan),
        _ => error(404, "not_found", "Not found", None),
    }
}

/// The parameter a plan request breaks the rules with.
fn plan_breach(body: &Value) -> Option<String> {
    if body.get("store") != Some(&json!(false)) {
        return Some("store".to_owned());
    }
    if body.get("stream") != Some(&json!(true)) {
        return Some("stream".to_owned());
    }
    if let Some(f) = PLAN_FORBIDDEN.iter().find(|f| body.get(**f).is_some()) {
        return Some((*f).to_owned());
    }
    let system = body
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|i| i.get("role").and_then(Value::as_str) == Some("system"));
    system.then(|| "input".to_owned())
}

fn responses(shared: &Shared, request: &Request, plan: bool) -> Answer {
    let body = request.json();
    if plan && let Some(param) = plan_breach(&body) {
        return error(
            400,
            "subscription_sharing_unsupported_capability",
            &format!("{param} is not supported with ChatGPT plan usage"),
            Some(&param),
        );
    }
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if model.is_empty() || body.get("input").is_none() {
        return error(400, "invalid_request", "model and input are required", None);
    }
    let reply = lock(&shared.replies)
        .pop_front()
        .unwrap_or_else(|| OpenAiResponse::text("OK").into());
    let n = {
        let mut n = lock(&shared.served);
        *n += 1;
        *n
    };
    match reply {
        OpenAiReply::Http(r) => Answer::Whole(r),
        OpenAiReply::Response(r) => {
            if body.get("stream").and_then(Value::as_bool) == Some(true) {
                Answer::Stream(Streamed {
                    frames: r.frames(&model, n),
                    cut_after: r.cut_after,
                    delay: r.delay,
                })
            } else {
                Answer::Whole(Reply::json(200, &r.whole(&model, n)))
            }
        }
    }
}

fn models(shared: &Shared, plan: bool) -> Answer {
    let body = if plan {
        let models: Vec<Value> = shared
            .config
            .plan_models
            .iter()
            .map(|(slug, name, vis)| json!({ "slug": slug, "display_name": name, "visibility": vis }))
            .collect();
        json!({ "models": models })
    } else {
        let data: Vec<Value> = shared
            .config
            .models
            .iter()
            .map(|id| json!({ "id": id, "object": "model", "created": 1_760_000_000, "owned_by": "openai" }))
            .collect();
        json!({ "object": "list", "data": data })
    };
    Answer::Whole(Reply::json(200, &body))
}
