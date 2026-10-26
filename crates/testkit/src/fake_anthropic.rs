//! A fake of Anthropic's Messages and Models APIs, for the `ai` crate and the routes and
//! scripts built on it.
//!
//! ```no_run
//! use serde_json::json;
//! use testkit::{AnthropicMessage, AnthropicReply, FakeAnthropic};
//!
//! let fake = FakeAnthropic::start();
//! // ai_settings.endpoints.anthropic = fake.base_url(); the key is fake.api_key().
//! fake.push(AnthropicMessage::text("Hello there"));
//! fake.push(AnthropicMessage::new().tool_use("toolu_1", "get_note", json!({"id": "01J"})));
//! fake.push(AnthropicMessage::new().refusal(Some("cyber"), Some("Not this one")));
//! fake.push(AnthropicReply::rate_limited(1));
//! // Only for a request whose body holds the text, as each of several running at once does:
//! fake.push_when("Research the venues", AnthropicMessage::text("The hall."));
//! // ... after the calls:
//! let sent = fake.message_requests(); // each request's headers and JSON body
//! ```
//!
//! `POST /v1/messages` answers the next queued reply, or [`AnthropicMessage::text`] `"OK"` when
//! none is queued. A reply queued with [`FakeAnthropic::push_when`] answers only a request whose
//! JSON body holds its text, so turns that run at once, such as sub agents, each get their own;
//! the first queued reply that may answer a request does. A queued [`AnthropicMessage`] is answered as the request asks: a stream of
//! `message_start`, `content_block_start`/`_delta`/`_stop` per block (text in small
//! `text_delta`s, tool inputs in several `input_json_delta`s, thinking in `thinking_delta`s and
//! one `signature_delta`), `message_delta` with the stop reason, stop details and usage, and
//! `message_stop`, when the body says `stream: true`; the whole message as JSON otherwise.
//!
//! It checks what the real API under `anthropic-version: 2026-10-21` checks: the `x-api-key`
//! (401 `authentication_error` otherwise; a missing one is accepted when built
//! [`FakeAnthropicBuilder::open`]), the version header, and no `temperature`, `top_p`
//! or `top_k`, which that version rejects (400 `invalid_request_error`). It also rejects
//! `fallbacks`, which TheVault never sends. `GET /v1/models` answers the configured models,
//! paginated with `limit` and `after_id`, newest first as Anthropic lists them.
//!
//! Images in a request (`image` blocks in Anthropic tool results, `input_image` items in OpenAI
//! tool outputs and messages) are checked, base64 with no line breaks and a PNG, JPEG, GIF or
//! WebP media type, else 400; `images()` gives each request's images decoded.
//!
//! It binds `127.0.0.1` on a free port. Every request is recorded, whatever it was answered.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::fake_http::{Answer, Listener, Streamed, frame, listen, lock, pieces};
use crate::fake_images::{FakeImage, anthropic_images};
use crate::standin::{Reply, Request};

/// The API key the fake accepts unless [`FakeAnthropicBuilder::api_key`] says otherwise.
pub const DEFAULT_ANTHROPIC_KEY: &str = "sk-ant-test-0123456789";

/// One answer to `POST /v1/messages`.
#[derive(Debug, Clone)]
pub enum AnthropicReply {
    /// A model message, streamed or whole as the request asks.
    Message(AnthropicMessage),
    /// A canned HTTP answer, such as an error.
    Http(Reply),
}

impl AnthropicReply {
    /// 429 `rate_limit_error` with `retry-after: <seconds>`.
    pub fn rate_limited(retry_after_secs: u64) -> AnthropicReply {
        AnthropicReply::error(
            429,
            "rate_limit_error",
            "Number of requests has exceeded your rate limit",
        )
        .header("retry-after", &retry_after_secs.to_string())
    }

    /// Anthropic's error body `{type: "error", error: {type, message}}` with `status`.
    pub fn error(status: u16, kind: &str, message: &str) -> AnthropicReply {
        AnthropicReply::Http(Reply::json(
            status,
            &json!({ "type": "error", "error": { "type": kind, "message": message } }),
        ))
    }

    /// Add a header to an [`AnthropicReply::Http`].
    pub fn header(self, name: &str, value: &str) -> AnthropicReply {
        match self {
            AnthropicReply::Http(r) => AnthropicReply::Http(r.header(name, value)),
            other => other,
        }
    }
}

impl From<AnthropicMessage> for AnthropicReply {
    fn from(m: AnthropicMessage) -> Self {
        AnthropicReply::Message(m)
    }
}

impl From<Reply> for AnthropicReply {
    fn from(r: Reply) -> Self {
        AnthropicReply::Http(r)
    }
}

/// A message the fake model answers, built block by block.
#[derive(Debug, Clone)]
pub struct AnthropicMessage {
    pub content: Vec<Value>,
    pub stop_reason: String,
    pub stop_details: Option<Value>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The size of each streamed text, thinking or input piece, in characters.
    pub chunk: usize,
    /// Close the stream after this many frames.
    pub cut_after: Option<usize>,
    /// Send an `error` event, `(type, message)`, after this many frames instead of the rest.
    pub error_after: Option<(usize, String, String)>,
    /// The pause after each streamed frame.
    pub delay: Duration,
}

impl Default for AnthropicMessage {
    fn default() -> Self {
        AnthropicMessage {
            content: Vec::new(),
            stop_reason: "end_turn".to_owned(),
            stop_details: None,
            input_tokens: 10,
            output_tokens: 5,
            chunk: 4,
            cut_after: None,
            error_after: None,
            delay: Duration::ZERO,
        }
    }
}

impl AnthropicMessage {
    /// An empty message that ends `end_turn`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A message of one text block.
    pub fn text(text: &str) -> Self {
        Self::new().with_text(text)
    }

    /// Add a text block.
    pub fn with_text(mut self, text: &str) -> Self {
        self.content.push(json!({ "type": "text", "text": text }));
        self
    }

    /// Add a thinking block with its signature.
    pub fn thinking(mut self, thinking: &str, signature: &str) -> Self {
        self.content
            .push(json!({ "type": "thinking", "thinking": thinking, "signature": signature }));
        self
    }

    /// Add a tool call, and end the message `tool_use`.
    pub fn tool_use(mut self, id: &str, name: &str, input: Value) -> Self {
        self.content
            .push(json!({ "type": "tool_use", "id": id, "name": name, "input": input }));
        self.stop_reason = "tool_use".to_owned();
        self
    }

    /// Add a tool call whose streamed input is `raw`, which need not be JSON. Whole, the message
    /// carries `raw` as a string input.
    pub fn tool_use_raw(mut self, id: &str, name: &str, raw: &str) -> Self {
        self.content
            .push(json!({ "type": "tool_use", "id": id, "name": name, "input": {}, "raw": raw }));
        self.stop_reason = "tool_use".to_owned();
        self
    }

    /// End with `reason`, such as `max_tokens`.
    pub fn stop(mut self, reason: &str) -> Self {
        self.stop_reason = reason.to_owned();
        self
    }

    /// End `refusal`, with `stop_details` naming the category and explanation.
    pub fn refusal(mut self, category: Option<&str>, explanation: Option<&str>) -> Self {
        self.stop_reason = "refusal".to_owned();
        self.stop_details = Some(json!({
            "type": "refusal", "category": category, "explanation": explanation,
        }));
        self
    }

    pub fn usage(mut self, input: u64, output: u64) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }

    /// Stream pieces of `n` characters.
    pub fn chunk(mut self, n: usize) -> Self {
        self.chunk = n.max(1);
        self
    }

    /// Break the stream off after `frames` frames.
    pub fn cut_after(mut self, frames: usize) -> Self {
        self.cut_after = Some(frames);
        self
    }

    /// Send an `error` event of `kind`, such as `overloaded_error`, after `frames` frames.
    pub fn error_after(mut self, frames: usize, kind: &str, message: &str) -> Self {
        self.error_after = Some((frames, kind.to_owned(), message.to_owned()));
        self
    }

    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    fn whole(&self, model: &str, id: &str) -> Value {
        let content: Vec<Value> = self
            .content
            .iter()
            .map(|b| {
                let mut b = b.clone();
                if let Some(raw) = b.as_object_mut().and_then(|o| o.remove("raw")) {
                    b["input"] = raw;
                }
                b
            })
            .collect();
        json!({
            "id": id, "type": "message", "role": "assistant", "model": model,
            "content": content, "stop_reason": self.stop_reason, "stop_sequence": null,
            "stop_details": self.stop_details,
            "usage": { "input_tokens": self.input_tokens, "output_tokens": self.output_tokens },
        })
    }

    fn frames(&self, model: &str, id: &str) -> Vec<String> {
        let mut f = vec![frame(
            "message_start",
            &json!({ "type": "message_start", "message": {
                "id": id, "type": "message", "role": "assistant", "model": model, "content": [],
                "stop_reason": null, "stop_sequence": null,
                "usage": { "input_tokens": self.input_tokens, "output_tokens": 1 },
            }}),
        )];
        f.push(frame("ping", &json!({ "type": "ping" })));
        for (index, b) in self.content.iter().enumerate() {
            let kind = b["type"].as_str().unwrap_or_default();
            let delta = |d: Value| {
                frame(
                    "content_block_delta",
                    &json!({ "type": "content_block_delta", "index": index, "delta": d }),
                )
            };
            let start = match kind {
                "text" => json!({ "type": "text", "text": "" }),
                "thinking" => json!({ "type": "thinking", "thinking": "", "signature": "" }),
                "tool_use" => {
                    json!({ "type": "tool_use", "id": b["id"], "name": b["name"], "input": {} })
                }
                _ => b.clone(),
            };
            f.push(frame(
                "content_block_start",
                &json!({ "type": "content_block_start", "index": index, "content_block": start }),
            ));
            match kind {
                "text" => {
                    for p in pieces(b["text"].as_str().unwrap_or_default(), self.chunk) {
                        f.push(delta(json!({ "type": "text_delta", "text": p })));
                    }
                }
                "thinking" => {
                    for p in pieces(b["thinking"].as_str().unwrap_or_default(), self.chunk) {
                        f.push(delta(json!({ "type": "thinking_delta", "thinking": p })));
                    }
                    f.push(delta(
                        json!({ "type": "signature_delta", "signature": b["signature"] }),
                    ));
                }
                "tool_use" => {
                    let raw = match b.get("raw").and_then(Value::as_str) {
                        Some(r) => r.to_owned(),
                        None => b["input"].to_string(),
                    };
                    for p in pieces(&raw, self.chunk) {
                        f.push(delta(
                            json!({ "type": "input_json_delta", "partial_json": p }),
                        ));
                    }
                }
                _ => {}
            }
            f.push(frame(
                "content_block_stop",
                &json!({ "type": "content_block_stop", "index": index }),
            ));
        }
        f.push(frame(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": self.stop_reason, "stop_sequence": null,
                    "stop_details": self.stop_details,
                },
                "usage": { "output_tokens": self.output_tokens },
            }),
        ));
        f.push(frame("message_stop", &json!({ "type": "message_stop" })));
        if let Some((after, kind, message)) = &self.error_after {
            f.truncate(*after);
            f.push(frame(
                "error",
                &json!({ "type": "error", "error": { "type": kind, "message": message } }),
            ));
        }
        f
    }
}

/// How a [`FakeAnthropic`] starts.
#[derive(Debug, Clone)]
pub struct FakeAnthropicBuilder {
    api_key: String,
    models: Vec<(String, String)>,
    page_size: usize,
    open: bool,
}

impl Default for FakeAnthropicBuilder {
    fn default() -> Self {
        FakeAnthropicBuilder {
            api_key: DEFAULT_ANTHROPIC_KEY.to_owned(),
            models: vec![
                ("claude-opus-5".to_owned(), "Claude Opus 5".to_owned()),
                ("claude-sonnet-5".to_owned(), "Claude Sonnet 5".to_owned()),
                ("claude-haiku-4-5".to_owned(), "Claude Haiku 4.5".to_owned()),
            ],
            page_size: 1000,
            open: false,
        }
    }
}

impl FakeAnthropicBuilder {
    /// Also answer requests that send no `x-api-key`, as a local server with no key does
    /// (ai D24). A wrong key is still refused.
    pub fn open(mut self) -> Self {
        self.open = true;
        self
    }

    /// The `x-api-key` the fake accepts.
    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = key.to_owned();
        self
    }

    /// The models `/v1/models` lists, `(id, display_name)`, newest first.
    pub fn models(mut self, models: &[(&str, &str)]) -> Self {
        self.models = models
            .iter()
            .map(|(i, d)| ((*i).to_owned(), (*d).to_owned()))
            .collect();
        self
    }

    /// The most models one page lists, whatever `limit` asks, to exercise paging.
    pub fn page_size(mut self, n: usize) -> Self {
        self.page_size = n.max(1);
        self
    }

    /// Bind a free port and start answering.
    pub fn start(self) -> FakeAnthropic {
        let shared = Arc::new(Shared {
            config: self,
            requests: Mutex::default(),
            replies: Mutex::default(),
            served: Mutex::new(0),
        });
        let s = shared.clone();
        let listener = listen("the fake Anthropic API", move |r| handle(&s, r));
        FakeAnthropic { listener, shared }
    }
}

struct Shared {
    config: FakeAnthropicBuilder,
    requests: Mutex<Vec<Request>>,
    /// Each reply with the text a request's body must hold for it, if any.
    replies: Mutex<VecDeque<(Option<String>, AnthropicReply)>>,
    served: Mutex<u64>,
}

/// A running fake Anthropic API. Dropping it stops the server.
pub struct FakeAnthropic {
    listener: Listener,
    shared: Arc<Shared>,
}

impl FakeAnthropic {
    pub fn builder() -> FakeAnthropicBuilder {
        FakeAnthropicBuilder::default()
    }

    /// Start with the defaults: [`DEFAULT_ANTHROPIC_KEY`] and three models.
    pub fn start() -> FakeAnthropic {
        Self::builder().start()
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener.addr
    }

    /// `http://127.0.0.1:<port>`: the value for `ai_settings.endpoints.anthropic`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.listener.addr)
    }

    /// The API key it accepts, for `ai_anthropic_api_key`.
    pub fn api_key(&self) -> &str {
        &self.shared.config.api_key
    }

    /// Queue the answer to a later `POST /v1/messages`, in order.
    pub fn push(&self, reply: impl Into<AnthropicReply>) {
        lock(&self.shared.replies).push_back((None, reply.into()));
    }

    /// Queue the answer to a later `POST /v1/messages` whose JSON body holds `text`.
    pub fn push_when(&self, text: &str, reply: impl Into<AnthropicReply>) {
        lock(&self.shared.replies).push_back((Some(text.to_owned()), reply.into()));
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.shared.requests).clone()
    }

    /// The `POST /v1/messages` requests received so far.
    pub fn message_requests(&self) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == "POST" && r.route() == "/v1/messages")
            .collect()
    }

    /// The images each `POST /v1/messages` request carried, in request order (ai D19).
    pub fn images(&self) -> Vec<Vec<FakeImage>> {
        self.message_requests()
            .iter()
            .map(|r| anthropic_images(&r.json()).unwrap_or_default())
            .collect()
    }

    pub fn clear_requests(&self) {
        lock(&self.shared.requests).clear();
    }
}

fn error(status: u16, kind: &str, message: &str) -> Answer {
    match AnthropicReply::error(status, kind, message) {
        AnthropicReply::Http(r) => Answer::Whole(r),
        AnthropicReply::Message(_) => unreachable!(),
    }
}

fn handle(shared: &Shared, request: &Request) -> Answer {
    lock(&shared.requests).push(request.clone());
    let key = request.header("x-api-key");
    let keyless = shared.config.open && key.is_none();
    if !keyless && key != Some(shared.config.api_key.as_str()) {
        return error(401, "authentication_error", "invalid x-api-key");
    }
    if request.header("anthropic-version").is_none() {
        return error(
            400,
            "invalid_request_error",
            "anthropic-version: header is required",
        );
    }
    match (request.method.as_str(), request.route()) {
        ("POST", "/v1/messages") => messages(shared, request),
        ("GET", "/v1/models") => models(shared, request),
        _ => error(404, "not_found_error", "Not found"),
    }
}

fn messages(shared: &Shared, request: &Request) -> Answer {
    let body = request.json();
    for field in ["temperature", "top_p", "top_k"] {
        if body.get(field).is_some() {
            return error(
                400,
                "invalid_request_error",
                &format!("{field}: is not supported by this API version"),
            );
        }
    }
    if body.get("fallbacks").is_some() {
        return error(
            400,
            "invalid_request_error",
            "fallbacks: needs a beta header",
        );
    }
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if model.is_empty() || body.get("max_tokens").is_none() || body.get("messages").is_none() {
        return error(
            400,
            "invalid_request_error",
            "model, max_tokens and messages are required",
        );
    }
    if let Err(e) = anthropic_images(&body) {
        return error(400, "invalid_request_error", &e);
    }
    let text = body.to_string();
    let reply = {
        let mut replies = lock(&shared.replies);
        replies
            .iter()
            .position(|(when, _)| when.as_ref().is_none_or(|w| text.contains(w.as_str())))
            .and_then(|i| replies.remove(i))
            .map(|(_, r)| r)
    }
    .unwrap_or_else(|| AnthropicMessage::text("OK").into());
    let id = {
        let mut n = lock(&shared.served);
        *n += 1;
        format!("msg_fake_{n}")
    };
    match reply {
        AnthropicReply::Http(r) => Answer::Whole(r),
        AnthropicReply::Message(m) => {
            if body.get("stream").and_then(Value::as_bool) == Some(true) {
                Answer::Stream(Streamed {
                    frames: m.frames(&model, &id),
                    cut_after: m.cut_after,
                    delay: m.delay,
                    headers: Vec::new(),
                })
            } else {
                Answer::Whole(Reply::json(200, &m.whole(&model, &id)))
            }
        }
    }
}

fn models(shared: &Shared, request: &Request) -> Answer {
    let all = &shared.config.models;
    let limit = request
        .query("limit")
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 1000)
        .min(shared.config.page_size);
    let start = match request.query("after_id") {
        Some(after) => all
            .iter()
            .position(|(id, _)| *id == after)
            .map_or(all.len(), |i| i + 1),
        None => 0,
    };
    let page: Vec<&(String, String)> = all.iter().skip(start).take(limit).collect();
    let data: Vec<Value> = page
        .iter()
        .map(|(id, name)| {
            json!({ "type": "model", "id": id, "display_name": name, "created_at": "2026-01-01T00:00:00Z" })
        })
        .collect();
    Answer::Whole(Reply::json(
        200,
        &json!({
            "data": data,
            "first_id": page.first().map(|m| &m.0),
            "last_id": page.last().map(|m| &m.0),
            "has_more": start + page.len() < all.len(),
        }),
    ))
}
