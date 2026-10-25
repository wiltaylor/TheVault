//! Calls to large language models with the user's own credentials (the AI spec's `crates/ai`
//! section, ai D1 to D12), through named accounts (accounts A1 to A13): an Anthropic Messages
//! client, an OpenAI Responses client used with an API key and with a ChatGPT plan token, an
//! OpenAI Chat Completions client for compatible services such as OpenRouter, Z.ai and Ollama
//! (ai D25), one incremental SSE parser under an async and a blocking transport, live model
//! lists, and credentials from the encrypted secrets.
//!
//! A call goes like this:
//!
//! ```no_run
//! # fn demo(state: &state::State) -> Result<(), ai::AiError> {
//! use std::time::{Duration, Instant};
//! use ai::{Accounts, Credentials, Request};
//!
//! let accounts = Accounts::load(state)?;
//! let account = accounts.pick(None, None)?; // the default account of Settings > AI
//! let cred = Credentials::resolve(state, account)?; // refreshes a ChatGPT token if due
//! let model = account.model().unwrap_or("claude-opus-5");
//! let req = Request::new(model, "Summarise: ...");
//! let began = Instant::now();
//! let result = ai::complete_blocking(&ai::blocking_client(), &cred, &req, None);
//! let line = ai::trail_for(&cred, &req.model, &result, began.elapsed());
//! # Ok(()) }
//! ```
//!
//! - Scripts (blocking): [`complete_blocking`] returns a [`Reply`], with [`Reply::json`] parsed
//!   and checked when the request has a schema; a refusal is `Err` with kind `refusal`. The
//!   ChatGPT plan always streams, read as it arrives; the API-key kinds answer whole.
//!   [`stream_blocking`] streams any account and hands each [`Event`] to a callback.
//! - The chat route (async): [`stream`] returns an [`EventStream`] of [`Event`]s, ending in
//!   `Done`, whose `stop` is a `Refusal` when the provider refused, so the partial answer can
//!   stay. Keep `Done`'s `provider_state` on the assistant [`Message`]; it is replayed when the
//!   next request goes to the same account over the same wire and to the same model, and dropped
//!   otherwise (ai D10, accounts A8). Tool calls arrive complete and checked against their
//!   schema, never on a turn cut off by `max_tokens` or a refusal (ai D5). [`complete`] collects
//!   a stream into a [`Reply`].
//! - Settings > AI: [`Accounts`] is the table of accounts, [`list_models`] (and
//!   [`list_models_blocking`]) read the live lists (ai D6), [`preselect`] picks a kind's default,
//!   [`Request::test`] is the Test button's request, [`Credentials::configured`] says whether an
//!   account is set up, and [`siwc`] holds the Sign in with ChatGPT pieces.
//! - Every call leaves [`trail_line`] (ai D12).
//! - A ChatGPT plan answer's usage headers are read into [`Reply::plan_usage`] and
//!   [`EventStream::plan_usage`] ([`PlanUsage`], accounts A13); the caller keeps the reading.
//!
//! [`Credentials::resolve`] and everything in [`siwc`] block; call them from
//! `tokio::task::spawn_blocking` in async code. Keys and tokens are [`Secret`]s, whose `Debug`
//! never shows them, and error messages are scrubbed of them.
//!
//! An account's base URL (ai D24, checked by [`check_url`]) is its own or its kind's. The ChatGPT
//! plan's is fixed, apart from the hidden test seam `ai_settings.endpoints`, where the testkit
//! fakes `FakeOpenAi` and `FakeSiwc` stand in for the real hosts. An account with its own base
//! URL needs no key: none sends no auth header. A 404 or 405 from an account's own base URL on
//! `/v1/messages`, `/v1/responses` or `/v1/chat/completions` is an `invalid` error saying the
//! service does not support that API ([`AiError::is_unsupported_api`]). Calls never probe.

mod accounts;
mod anthropic;
mod chat;
mod credentials;
mod error;
pub mod html;
mod http;
mod openai;
pub mod schema;
mod settings;
pub mod siwc;
pub mod sse;
mod trail;
mod turn;
mod types;
pub mod web;

use std::collections::VecDeque;
use std::io::Read;
use std::time::Instant;

use serde_json::Value;

pub use accounts::{
    ACCOUNTS, Account, AccountContext, AccountInput, Accounts, HEADERS_MAX, NAME_MAX, PlanUsage,
    REFUSED_HEADERS, UsageWindow, chatgpt_secret, check_headers, check_name, default_base,
    default_window, key_secret, no_account, usage_secret,
};
pub use anthropic::{
    MAX_TOKENS as ANTHROPIC_MAX_TOKENS, STREAM_MAX_TOKENS, VERSION as ANTHROPIC_VERSION,
};
pub use credentials::{
    ChatGptAuth, Credential, Credentials, EXPIRED_TITLE, REFRESH_MARGIN_MS, SETTINGS_URL, Secret,
};
pub use error::{AiError, ErrorKind};
pub use http::{blocking_client, client};
pub use settings::{
    ANTHROPIC_BASE, ANTHROPIC_DEFAULT_MODEL, CHATGPT_AUTH_BASE, Confirm, Context, Endpoints,
    OPENAI_BASE, SETTINGS, SYSTEM_PROMPT_MAX, Settings, Web, api_url, check_url,
};
pub use trail::{Outcome, trail_for, trail_line};
pub use types::{
    Content, Effort, Event, ImageData, Kind, Message, Model, Part, ProviderState, Reply, Request,
    Role, Stop, ToolCall, ToolSpec, Usage, Wire,
};

use http::Call;
use sse::{SseFrame, SseParser};
use turn::Collector;

/// The request as an HTTP call to `cred`'s account.
fn inference_call(cred: &Credential, req: &Request, stream: bool) -> Result<Call, AiError> {
    if req.model.trim().is_empty() {
        return Err(AiError::not_configured(format!(
            "no model is chosen for the AI account {}: pick one in Settings > AI",
            cred.name
        ))
        .on(&cred.name, ""));
    }
    Ok(match cred.kind {
        Kind::Anthropic => Call::new(cred, &req.model, "/v1/messages")
            .post(anthropic::body(cred, req, stream), stream),
        Kind::OpenAiCompatible => Call::new(cred, &req.model, chat::PATH)
            .post(chat::body(cred, req, stream, false), stream),
        Kind::OpenAi => Call::new(cred, &req.model, "/v1/responses")
            .post(openai::body(cred, req, false, stream), stream),
        Kind::ChatGpt => Call::new(cred, &req.model, "/v1/responses")
            .post(openai::body(cred, req, true, true), true),
    })
}

/// The second try of a Chat Completions request with a schema that the service refused with a
/// 400: `json_object` with the schema in the system text (ai D25). `None` when `e` is not that.
fn json_object_call(cred: &Credential, req: &Request, stream: bool, e: &AiError) -> Option<Call> {
    (cred.chat_completions() && req.schema.is_some() && e.status == Some(400)).then(|| {
        Call::new(cred, &req.model, chat::PATH).post(chat::body(cred, req, stream, true), stream)
    })
}

/// Send the request (async), with the `json_object` second try when it applies. Returns the
/// call that was answered.
async fn send_request(
    http: &reqwest::Client,
    cred: &Credential,
    req: &Request,
    stream: bool,
) -> Result<(reqwest::Response, Call), AiError> {
    let call = inference_call(cred, req, stream)?;
    match http::send(http, &call).await {
        Ok(resp) => Ok((resp, call)),
        Err(e) => match json_object_call(cred, req, stream, &e) {
            Some(again) => Ok((http::send(http, &again).await?, again)),
            None => Err(e),
        },
    }
}

/// [`send_request`], blocking.
fn send_request_blocking(
    http: &reqwest::blocking::Client,
    cred: &Credential,
    req: &Request,
    stream: bool,
    deadline: Option<Instant>,
) -> Result<(reqwest::blocking::Response, Call), AiError> {
    let call = inference_call(cred, req, stream)?;
    match http::send_blocking(http, &call, deadline) {
        Ok(resp) => Ok((resp, call)),
        Err(e) => match json_object_call(cred, req, stream, &e) {
            Some(again) => Ok((http::send_blocking(http, &again, deadline)?, again)),
            None => Err(e),
        },
    }
}

/// The decoder of a provider's stream.
enum Decoder {
    Anthropic(anthropic::StreamDecoder),
    OpenAi(openai::StreamDecoder),
    Chat(chat::StreamDecoder),
}

/// Bytes in, events out: the parser and decoder both transports share.
struct Pump {
    parser: SseParser,
    decoder: Decoder,
    pending: VecDeque<Event>,
    failed: Option<AiError>,
    name: String,
}

impl Pump {
    fn new(cred: &Credential, req: &Request) -> Pump {
        Pump {
            parser: SseParser::new(),
            decoder: match cred.wire() {
                Wire::Messages => Decoder::Anthropic(anthropic::StreamDecoder::new(cred, req)),
                Wire::ChatCompletions => Decoder::Chat(chat::StreamDecoder::new(cred, req)),
                Wire::Responses => Decoder::OpenAi(openai::StreamDecoder::new(cred, req)),
            },
            pending: VecDeque::new(),
            failed: None,
            name: cred.name.clone(),
        }
    }

    fn done(&self) -> bool {
        match &self.decoder {
            Decoder::Anthropic(d) => d.done,
            Decoder::OpenAi(d) => d.done,
            Decoder::Chat(d) => d.done,
        }
    }

    fn usage(&self) -> Usage {
        match &self.decoder {
            Decoder::Anthropic(d) => d.usage(),
            Decoder::OpenAi(d) => d.usage(),
            Decoder::Chat(d) => d.usage(),
        }
    }

    fn bytes(&mut self, chunk: &[u8]) {
        for frame in self.parser.feed(chunk) {
            if self.done() || self.failed.is_some() {
                return;
            }
            if let Err(e) = self.frame(&frame) {
                self.failed = Some(e);
            }
        }
    }

    fn frame(&mut self, frame: &SseFrame) -> Result<(), AiError> {
        if frame.data.trim() == "[DONE]" {
            return match &mut self.decoder {
                Decoder::Chat(d) => d.finish(&mut self.pending),
                _ => Ok(()),
            };
        }
        let data: Value = serde_json::from_str(&frame.data).map_err(|e| {
            AiError::new(
                ErrorKind::Provider,
                format!("{} sent an unreadable event: {e}", self.name),
            )
            .with_usage(self.usage())
        })?;
        match &mut self.decoder {
            Decoder::Anthropic(d) => d.frame(&data, &mut self.pending),
            Decoder::OpenAi(d) => d.frame(&data, &mut self.pending),
            Decoder::Chat(d) => d.frame(&data, &mut self.pending),
        }
    }

    /// The body ended: an error unless the turn did too.
    fn end(&mut self) {
        self.parser.finish();
        // A Chat Completions service that ends the body after the finish_reason without
        // `[DONE]` still ended the turn.
        if let Decoder::Chat(d) = &mut self.decoder
            && d.finished()
            && self.failed.is_none()
            && let Err(e) = d.finish(&mut self.pending)
        {
            self.failed = Some(e);
        }
        if !self.done() && self.failed.is_none() {
            let last = match self.decoder {
                Decoder::Anthropic(_) => "message_stop",
                Decoder::OpenAi(_) => "response.completed",
                Decoder::Chat(_) => "[DONE]",
            };
            self.failed = Some(
                AiError::new(
                    ErrorKind::Provider,
                    format!("{} stopped answering before {last}", self.name),
                )
                .with_usage(self.usage()),
            );
        }
    }
}

/// A streamed call's events. Dropping it closes the connection, which stops the provider.
pub struct EventStream {
    resp: Option<reqwest::Response>,
    call: Call,
    pump: Pump,
    over: bool,
    plan_usage: Option<PlanUsage>,
}

impl EventStream {
    /// The usage the ChatGPT plan reported in the answer's headers, if it did (accounts A13).
    pub fn plan_usage(&self) -> Option<&PlanUsage> {
        self.plan_usage.as_ref()
    }

    /// The next event, or `None` once `Done` or an error has been returned.
    pub async fn next(&mut self) -> Option<Result<Event, AiError>> {
        loop {
            if let Some(e) = self.pump.pending.pop_front() {
                return Some(Ok(e));
            }
            if self.over {
                return None;
            }
            if let Some(e) = self.pump.failed.take() {
                self.over = true;
                self.resp = None;
                return Some(Err(self.call.fail(e)));
            }
            if self.pump.done() {
                self.over = true;
                self.resp = None;
                continue;
            }
            let Some(resp) = self.resp.as_mut() else {
                self.over = true;
                continue;
            };
            match resp.chunk().await {
                Ok(Some(bytes)) => self.pump.bytes(&bytes),
                Ok(None) => self.pump.end(),
                Err(e) => {
                    let usage = self.pump.usage();
                    self.pump.failed = Some(http::body_error(&self.call, &e).with_usage(usage));
                }
            }
        }
    }

    /// The events as a `futures` stream.
    pub fn into_stream(self) -> impl futures_util::Stream<Item = Result<Event, AiError>> + Send {
        futures_util::stream::unfold(self, |mut s| async move {
            let item = s.next().await?;
            Some((item, s))
        })
    }

    /// Read every event into a [`Reply`], as [`complete`] does.
    pub async fn collect(mut self, cred: &Credential, req: &Request) -> Result<Reply, AiError> {
        let mut c = Collector::new(cred, req);
        while let Some(e) = self.next().await {
            c.event(&e?);
        }
        let mut reply = c.finish()?;
        reply.plan_usage = self.plan_usage.take();
        Ok(reply)
    }
}

/// Start a streamed call (async). The request is sent, retried on a 429, a 5xx or a connection
/// failure as [`http`] describes, and the answer's events are read as they arrive.
pub async fn stream(
    http: &reqwest::Client,
    cred: &Credential,
    req: &Request,
) -> Result<EventStream, AiError> {
    let (resp, call) = send_request(http, cred, req, true).await?;
    Ok(EventStream {
        plan_usage: plan_usage(cred, resp.headers()),
        resp: Some(resp),
        call,
        pump: Pump::new(cred, req),
        over: false,
    })
}

/// A whole call (async): [`stream`], collected. A refusal is an error (ai D11).
pub async fn complete(
    http: &reqwest::Client,
    cred: &Credential,
    req: &Request,
) -> Result<Reply, AiError> {
    stream(http, cred, req).await?.collect(cred, req).await
}

/// A whole call, blocking, giving up at `deadline`. A ChatGPT plan call streams, as it must
/// (ai D4), and is read as it arrives; the others are answered whole. A refusal is an error
/// (ai D11).
pub fn complete_blocking(
    http: &reqwest::blocking::Client,
    cred: &Credential,
    req: &Request,
    deadline: Option<Instant>,
) -> Result<Reply, AiError> {
    if cred.kind == Kind::ChatGpt {
        return stream_blocking(http, cred, req, deadline, |_| {});
    }
    let (resp, call) = send_request_blocking(http, cred, req, false, deadline)?;
    let text = resp.text().map_err(|e| http::body_error(&call, &e))?;
    let body: Value = serde_json::from_str(&text).map_err(|e| {
        call.fail(AiError::new(
            ErrorKind::Provider,
            format!("{} sent an unreadable answer: {e}", call.name),
        ))
    })?;
    let events = match cred.wire() {
        Wire::Messages => anthropic::parse_message(cred, req, &body),
        Wire::ChatCompletions => chat::parse_completion(cred, req, &body),
        Wire::Responses => openai::parse_response(cred, req, &body),
    }
    .map_err(|e| call.fail(e))?;
    let mut c = Collector::new(cred, req);
    for e in &events {
        c.event(e);
    }
    c.finish()
}

/// A streamed call, blocking, handing each event to `on_event` as it arrives and returning the
/// collected [`Reply`].
pub fn stream_blocking(
    http: &reqwest::blocking::Client,
    cred: &Credential,
    req: &Request,
    deadline: Option<Instant>,
    mut on_event: impl FnMut(&Event),
) -> Result<Reply, AiError> {
    let (mut resp, call) = send_request_blocking(http, cred, req, true, deadline)?;
    let usage = plan_usage(cred, resp.headers());
    let mut pump = Pump::new(cred, req);
    let mut c = Collector::new(cred, req);
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        while let Some(e) = pump.pending.pop_front() {
            on_event(&e);
            c.event(&e);
        }
        if let Some(e) = pump.failed.take() {
            return Err(call.fail(e));
        }
        if pump.done() {
            break;
        }
        match resp.read(&mut buf) {
            Ok(0) => pump.end(),
            Ok(n) => pump.bytes(&buf[..n]),
            Err(e) => {
                let timeout =
                    e.kind() == std::io::ErrorKind::TimedOut || e.to_string().contains("timed out");
                let usage = pump.usage();
                return Err(http::read_error(&call, &e, timeout).with_usage(usage));
            }
        }
    }
    let mut reply = c.finish()?;
    reply.plan_usage = usage;
    Ok(reply)
}

/// The usage a ChatGPT plan answer reports in its headers (accounts A13); none for the other
/// kinds.
fn plan_usage(cred: &Credential, headers: &reqwest::header::HeaderMap) -> Option<PlanUsage> {
    (cred.kind == Kind::ChatGpt)
        .then(|| PlanUsage::from_headers(headers, credentials::now_ms()))
        .flatten()
}

/// The most pages of Anthropic's model list read.
const MAX_MODEL_PAGES: usize = 20;

fn models_call(cred: &Credential, after: Option<&str>) -> Call {
    let path = match (cred.kind, after) {
        (Kind::Anthropic, Some(a)) => format!("/v1/models?limit=1000&after_id={a}"),
        (Kind::Anthropic, None) => "/v1/models?limit=1000".to_owned(),
        _ => "/v1/models".to_owned(),
    };
    Call::new(cred, "", &path)
}

fn unreadable(call: &Call, e: impl std::fmt::Display) -> AiError {
    call.fail(AiError::new(
        ErrorKind::Provider,
        format!("{} sent an unreadable model list: {e}", call.name),
    ))
}

/// The models `cred`'s provider offers now (async, ai D6).
pub async fn list_models(http: &reqwest::Client, cred: &Credential) -> Result<Vec<Model>, AiError> {
    let mut models = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_MODEL_PAGES {
        let call = models_call(cred, after.as_deref());
        let page: Value = http::send(http, &call)
            .await?
            .json()
            .await
            .map_err(|e| unreadable(&call, e))?;
        if cred.kind != Kind::Anthropic {
            return Ok(openai::models(cred.kind, &page));
        }
        let (more, next) = anthropic::models_page(&page);
        models.extend(more);
        match next {
            Some(n) => after = Some(n),
            None => break,
        }
    }
    Ok(models)
}

/// [`list_models`], blocking.
pub fn list_models_blocking(
    http: &reqwest::blocking::Client,
    cred: &Credential,
    deadline: Option<Instant>,
) -> Result<Vec<Model>, AiError> {
    let mut models = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_MODEL_PAGES {
        let call = models_call(cred, after.as_deref());
        let page: Value = http::send_blocking(http, &call, deadline)?
            .json()
            .map_err(|e| unreadable(&call, e))?;
        if cred.kind != Kind::Anthropic {
            return Ok(openai::models(cred.kind, &page));
        }
        let (more, next) = anthropic::models_page(&page);
        models.extend(more);
        match next {
            Some(n) => after = Some(n),
            None => break,
        }
    }
    Ok(models)
}

/// The model a `kind` account uses when the user has picked none (ai D6, accounts A5):
/// [`ANTHROPIC_DEFAULT_MODEL`] when Anthropic lists it, and the first model listed for the other
/// kinds. No other model id is hard-coded.
pub fn preselect(kind: Kind, models: &[Model]) -> Option<&Model> {
    match kind {
        Kind::Anthropic => models.iter().find(|m| m.id == ANTHROPIC_DEFAULT_MODEL),
        _ => models.first(),
    }
}

/// The model a call with `cred` uses when no request, agent or account names one: the live
/// list's [`preselect`], read with a blocking call. `not_configured` when the list names none.
pub fn default_model_blocking(
    http: &reqwest::blocking::Client,
    cred: &Credential,
    deadline: Option<Instant>,
) -> Result<String, AiError> {
    let models = list_models_blocking(http, cred, deadline)?;
    preselect(cred.kind, &models)
        .map(|m| m.id.clone())
        .ok_or_else(|| {
            AiError::not_configured(format!(
                "no model is chosen for the AI account {}: pick one in Settings > AI",
                cred.name
            ))
            .on(&cred.name, "")
        })
}

/// The chat panel's default system prompt (ai D14): fixed, versioned text, so it stays the same
/// from one request to the next and the providers' prompt caching applies. Settings > AI can
/// replace it (ai D17, [`Settings::chat_prompt`]). The open note, when there is one, is added
/// after whichever is in use.
pub const CHAT_PROMPT: &str = include_str!("chat_prompt.md");
