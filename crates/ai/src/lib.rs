//! Calls to large language models with the user's own credentials (the AI spec's `crates/ai`
//! section, ai D1 to D12): an Anthropic Messages client, an OpenAI Responses client used with an
//! API key and with a ChatGPT plan token, an OpenAI Chat Completions client for compatible
//! services that lack the Responses API (ai D25), one incremental SSE parser under an async and
//! a blocking transport, live model lists, and credentials from the encrypted secrets.
//!
//! A call goes like this:
//!
//! ```no_run
//! # fn demo(state: &state::State) -> Result<(), ai::AiError> {
//! use std::time::{Duration, Instant};
//! use ai::{Credentials, Provider, Request, Settings};
//!
//! let settings = Settings::load(state)?;
//! let (provider, model) = settings.pick(None, None)?; // the defaults of Settings > AI
//! let cred = Credentials::resolve(state, provider)?; // refreshes a ChatGPT token if due
//! let req = Request::new(provider, model, "Summarise: ...");
//! let began = Instant::now();
//! let result = ai::complete_blocking(&ai::blocking_client(), &cred, &req, None);
//! let host = cred.custom_host.as_deref(); // a custom API URL's host, for the trail
//! let line = ai::trail_for(provider, &req.model, &result, began.elapsed(), host);
//! # Ok(()) }
//! ```
//!
//! - Scripts (blocking): [`complete_blocking`] returns a [`Reply`], with [`Reply::json`] parsed
//!   and checked when the request has a schema; a refusal is `Err` with kind `refusal`. The
//!   ChatGPT plan always streams, read as it arrives; the API-key providers answer whole.
//!   [`stream_blocking`] streams any provider and hands each [`Event`] to a callback.
//! - The chat route (async): [`stream`] returns an [`EventStream`] of [`Event`]s, ending in
//!   `Done`, whose `stop` is a `Refusal` when the provider refused, so the partial answer can
//!   stay. Keep `Done`'s `provider_state` on the assistant [`Message`]; it is replayed when the
//!   next request uses the same provider and model and dropped otherwise (ai D10). Tool calls
//!   arrive complete and checked against their schema, never on a turn cut off by `max_tokens`
//!   or a refusal (ai D5). [`complete`] collects a stream into a [`Reply`].
//! - Settings > AI: [`list_models`] (and [`list_models_blocking`]) read the live lists (ai D6),
//!   [`preselect`] picks Anthropic's default, [`Request::test`] is the Test button's request,
//!   [`Credentials::configured`] says whether a provider is set up, and [`siwc`] holds the Sign
//!   in with ChatGPT pieces.
//! - Every call leaves [`trail_line`] (ai D12).
//!
//! [`Credentials::resolve`] and everything in [`siwc`] block; call them from
//! `tokio::task::spawn_blocking` in async code. Keys and tokens are [`Secret`]s, whose `Debug`
//! never shows them, and error messages are scrubbed of them.
//!
//! Every base URL comes from `ai_settings.endpoints` ([`Endpoints`]): the Anthropic and OpenAI
//! API URLs of Settings > AI (ai D24, checked by [`check_url`]), and the test seam, where the
//! testkit fakes `FakeAnthropic`, `FakeOpenAi` and `FakeSiwc` stand in for the real hosts. With
//! a custom API URL the key is optional: none sends no auth header. A 404 or 405 from a custom
//! URL on `/v1/messages`, `/v1/responses` or `/v1/chat/completions` is an `invalid` error saying
//! the service does not support that API ([`AiError::is_unsupported_api`]).
//!
//! An OpenAI credential with a custom API URL speaks the API `endpoints.openai_api` names
//! ([`OpenAiApi`], ai D25): Responses by default, or Chat Completions. The official OpenAI API
//! and the ChatGPT plan always use Responses. Calls never probe: Settings > AI's Test does.

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

pub use anthropic::{
    MAX_TOKENS as ANTHROPIC_MAX_TOKENS, STREAM_MAX_TOKENS, VERSION as ANTHROPIC_VERSION,
};
pub use credentials::{
    ANTHROPIC_API_KEY, CHATGPT, ChatGptAuth, Credential, Credentials, EXPIRED_TITLE,
    OPENAI_API_KEY, REFRESH_MARGIN_MS, SETTINGS_URL, Secret, key_secret,
};
pub use error::{AiError, ErrorKind};
pub use http::{blocking_client, client};
pub use settings::{
    ANTHROPIC_BASE, ANTHROPIC_DEFAULT_MODEL, CHATGPT_AUTH_BASE, Confirm, Context, Endpoints,
    Models, OPENAI_BASE, OpenAiApi, SETTINGS, SYSTEM_PROMPT_MAX, Settings, Web, api_url, check_url,
};
pub use trail::{Outcome, trail_for, trail_line};
pub use types::{
    Content, Effort, Event, ImageData, Message, Model, Part, Provider, ProviderState, Reply,
    Request, Role, Stop, ToolCall, ToolSpec, Usage,
};

use http::Call;
use sse::{SseFrame, SseParser};
use turn::Collector;

/// The request as an HTTP call to `cred`'s provider.
fn inference_call(cred: &Credential, req: &Request, stream: bool) -> Result<Call, AiError> {
    if req.provider != cred.provider {
        return Err(AiError::invalid(format!(
            "a {} request cannot use the {} credential",
            req.provider, cred.provider
        )));
    }
    if req.model.trim().is_empty() {
        return Err(AiError::not_configured(format!(
            "no {} model is chosen: pick one in Settings > AI",
            req.provider.label()
        ))
        .on(req.provider, ""));
    }
    Ok(match cred.provider {
        Provider::Anthropic => {
            Call::new(cred, &req.model, "/v1/messages").post(anthropic::body(req, stream), stream)
        }
        Provider::OpenAi if cred.chat_completions() => {
            Call::new(cred, &req.model, chat::PATH).post(chat::body(req, stream, false), stream)
        }
        Provider::OpenAi => Call::new(cred, &req.model, "/v1/responses")
            .post(openai::body(req, false, stream), stream),
        Provider::ChatGpt => {
            Call::new(cred, &req.model, "/v1/responses").post(openai::body(req, true, true), true)
        }
    })
}

/// The second try of a Chat Completions request with a schema that the service refused with a
/// 400: `json_object` with the schema in the system text (ai D25). `None` when `e` is not that.
fn json_object_call(cred: &Credential, req: &Request, stream: bool, e: &AiError) -> Option<Call> {
    (cred.chat_completions() && req.schema.is_some() && e.status == Some(400)).then(|| {
        Call::new(cred, &req.model, chat::PATH).post(chat::body(req, stream, true), stream)
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
    provider: Provider,
}

impl Pump {
    fn new(cred: &Credential, req: &Request) -> Pump {
        Pump {
            parser: SseParser::new(),
            decoder: match req.provider {
                Provider::Anthropic => Decoder::Anthropic(anthropic::StreamDecoder::new(req)),
                Provider::OpenAi if cred.chat_completions() => {
                    Decoder::Chat(chat::StreamDecoder::new(req))
                }
                Provider::OpenAi | Provider::ChatGpt => {
                    Decoder::OpenAi(openai::StreamDecoder::new(req))
                }
            },
            pending: VecDeque::new(),
            failed: None,
            provider: req.provider,
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
                format!("{} sent an unreadable event: {e}", self.provider.label()),
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
                    format!("{} stopped answering before {last}", self.provider.label()),
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
}

impl EventStream {
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
    pub async fn collect(mut self, req: &Request) -> Result<Reply, AiError> {
        let mut c = Collector::new(req);
        while let Some(e) = self.next().await {
            c.event(&e?);
        }
        c.finish()
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
    stream(http, cred, req).await?.collect(req).await
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
    if cred.provider == Provider::ChatGpt {
        return stream_blocking(http, cred, req, deadline, |_| {});
    }
    let (resp, call) = send_request_blocking(http, cred, req, false, deadline)?;
    let text = resp.text().map_err(|e| http::body_error(&call, &e))?;
    let body: Value = serde_json::from_str(&text).map_err(|e| {
        call.fail(AiError::new(
            ErrorKind::Provider,
            format!("{} sent an unreadable answer: {e}", call.provider.label()),
        ))
    })?;
    let events = match cred.provider {
        Provider::Anthropic => anthropic::parse_message(req, &body),
        Provider::OpenAi if cred.chat_completions() => chat::parse_completion(req, &body),
        _ => openai::parse_response(req, &body),
    }
    .map_err(|e| call.fail(e))?;
    let mut c = Collector::new(req);
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
    let mut pump = Pump::new(cred, req);
    let mut c = Collector::new(req);
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
    c.finish()
}

/// The most pages of Anthropic's model list read.
const MAX_MODEL_PAGES: usize = 20;

fn models_call(cred: &Credential, after: Option<&str>) -> Call {
    let path = match (cred.provider, after) {
        (Provider::Anthropic, Some(a)) => format!("/v1/models?limit=1000&after_id={a}"),
        (Provider::Anthropic, None) => "/v1/models?limit=1000".to_owned(),
        _ => "/v1/models".to_owned(),
    };
    Call::new(cred, "", &path)
}

fn unreadable(call: &Call, e: impl std::fmt::Display) -> AiError {
    call.fail(AiError::new(
        ErrorKind::Provider,
        format!(
            "{} sent an unreadable model list: {e}",
            call.provider.label()
        ),
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
        if cred.provider != Provider::Anthropic {
            return Ok(openai::models(cred.provider, &page));
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
        if cred.provider != Provider::Anthropic {
            return Ok(openai::models(cred.provider, &page));
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

/// The model Settings > AI preselects for `provider` when the user has picked none:
/// [`ANTHROPIC_DEFAULT_MODEL`] when Anthropic lists it, and nothing otherwise, since no other
/// model id is hard-coded (ai D6).
pub fn preselect(provider: Provider, models: &[Model]) -> Option<&Model> {
    match provider {
        Provider::Anthropic => models.iter().find(|m| m.id == ANTHROPIC_DEFAULT_MODEL),
        _ => None,
    }
}

/// The chat panel's default system prompt (ai D14): fixed, versioned text, so it stays the same
/// from one request to the next and the providers' prompt caching applies. Settings > AI can
/// replace it (ai D17, [`Settings::chat_prompt`]). The open note, when there is one, is added
/// after whichever is in use.
pub const CHAT_PROMPT: &str = include_str!("chat_prompt.md");
