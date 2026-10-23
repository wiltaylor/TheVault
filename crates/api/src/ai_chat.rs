//! `POST /api/ai/chat`: the chat panel's conversation turn (ai D10, D11, D14).
//!
//! The body is `{ provider?, model?, messages, note_id? }`: the transcript so far, ending with the
//! user's new message, in the neutral form of [`ai::Message`]. Provider and model default to
//! Settings > AI's. `note_id` names the note open beside the panel: its id, type and title (never
//! its body) join the system prompt, so "this note" resolves through `get_note`.
//!
//! The answer is a server-sent event stream. The server runs the tool loop, at most
//! [`MAX_ROUNDS`] model calls per user turn, over read-only tools; `propose_edit`,
//! `propose_create`, `propose_archive`, `propose_delete`, `propose_type_change`,
//! `propose_taxonomy_change` and `propose_taxonomy_delete` check a change and hand it to the user
//! (ai D14, D18, D20, D31). When Settings > AI turns a kind's confirmation off and the caller's
//! scope allows the write (`write` for notes, `admin` for types and taxonomies), the tool makes
//! the change itself with [`apply_change`], as the caller, through the writer calls the panel's
//! Apply reaches, and sends `applied` in place of the proposal (ai D22). Nothing here ever purges
//! the trash (ai D21). With a Brave Search key stored, `web_search` searches the web
//! (`crate::brave`, ai D26). Events:
//!
//! | Event | Data |
//! |---|---|
//! | `text` | `{delta}`: answer text as it arrives |
//! | `tool` | `{id, name, input}`: a tool the model called, before it runs |
//! | `tool_result` | `{id, summary, is_error}`: `summary` holds the ids and titles returned (`notes`), never bodies, a `message` on an error, for `read_asset` the `asset` read (id, filename, media type, size, kind), never its bytes, and for `fetch_url` and `download_asset` the `web` address read (url, host, media type, size, kind, and a page's title or a download's filename and asset id), never its content (ai D27) |
//! | `proposal` | a note change for the user to apply, by `op` (ai D14, D20): `update` `{id, op, note_id, title, base_version, before, after, ops}`; `create` `{id, op, type, title, after}`; `archive`, `unarchive` and `delete` `{id, op, note_id, title, base_version}`; and a file to store, `download` `{id, op, url, filename, media_type, size}` (ai D27) |
//! | `type_proposal` | `{id, name, exists, before, after, issues}`: a type file's new WCL for the user to apply (ai D18) |
//! | `taxonomy_proposal` | a taxonomy change for the user to apply (ai D31), `kind` always `taxonomy`: `create` and `change` `{id, kind, op, name, exists, before, after, renames, issues, affected}`, `renames` a list of `{from, to}` term paths and `affected` the number of notes they rewrite; `delete` `{id, kind, op, name, before}` |
//! | `applied` | a change the tool made itself (ai D22): `{id, op, note_id, title, version}` for `update`, `create`, `archive`, `unarchive` and `delete` (whose version is the one that went to the trash); `{id, op, type_name}` for `type_change` and `type_create`; `{id, op, taxonomy_name, notes}` for `taxonomy_change`, `taxonomy_create` and `taxonomy_delete` (ai D31); `{id, op, asset_id, filename, media_type, size, url}` for `download` (ai D27) |
//! | `usage` | `{provider, model, input, output}`: one model call's tokens |
//! | `refusal` | `{provider, model, category, explanation, messages}`: the provider refused; ends the stream |
//! | `error` | `{kind, message, messages}`: the turn failed; ends the stream |
//! | `done` | `{stop, messages}`: the turn is over |
//!
//! `messages` is the turn's new transcript items, for the client to append and send back next
//! turn: assistant messages with their `provider_state`, and the user messages carrying tool
//! results. They are never edited afterwards. A comment goes out every [`HEARTBEAT`]. When the
//! client goes away the loop stops, and the provider call in flight is dropped, which closes its
//! connection. The turns form (`crate::ai_turns`, ai D23) runs its turns through the same
//! [`start`], and `POST /api/ai/apply` applies a proposal with [`apply_change`].

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ai::{
    AiError, Content, Credential, Credentials, ErrorKind, Event as AiEvent, Message, Outcome,
    Provider, ProviderState, Request, Role, Settings, Stop, ToolCall, ToolSpec, Usage,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, HeaderName};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Extension, Json};
use script_core::Vault;
use serde::Deserialize;
use serde_json::{Value, json};
use state::{Author, Scope};
use tokio::sync::{mpsc, watch};
use writer::{Input, Op, TypeChange, Writer, doc_to_json};

use crate::ai_taxonomies::apply_taxonomy;
use crate::auth::{Caller, GrantedScope};
use crate::events::HEARTBEAT;
use crate::{ApiError, ai_assets};

/// The most model calls one user turn makes (ai D14).
pub const MAX_ROUNDS: usize = 12;

/// The most rows `search_notes` returns.
pub const SEARCH_LIMIT: usize = 20;

/// The most rows `query_notes` returns.
pub const QUERY_LIMIT: usize = 50;

/// The longest text `get_note` returns in one value before cutting it.
pub const BODY_LIMIT: usize = 40_000;

/// The longest tool result sent back to the model.
pub const RESULT_LIMIT: usize = 100_000;

/// Fields the server sets itself, left out of a proposal's changes.
const SERVER_FIELDS: [&str; 2] = ["created", "updated"];

/// The request body.
#[derive(Debug, Deserialize)]
pub struct ChatBody {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    messages: Vec<Message>,
    #[serde(default)]
    note_id: Option<String>,
}

/// The one HTTP client chat calls share.
fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(ai::client)
}

/// `POST /api/ai/chat`.
pub async fn chat(
    State(w): State<Arc<Writer>>,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Json(body): Json<ChatBody>,
) -> Result<impl IntoResponse, ApiError> {
    // No one stops this form of turn but the client, by going away.
    let (_, never) = watch::channel(false);
    let (rx, _) = start(w, author, scope, body, never)?;
    Ok(sse(receive(rx)))
}

/// Check `body` and start its turn, which sends its events to the receiver returned and stops
/// when the receiver goes or `stop` turns true. The flag returned turns false when the turn is
/// over. Both chat forms start their turns here.
pub(crate) fn start(
    w: Arc<Writer>,
    author: Author,
    scope: Scope,
    body: ChatBody,
    stop: watch::Receiver<bool>,
) -> Result<(mpsc::Receiver<Event>, Arc<AtomicBool>), ApiError> {
    let provider = match body.provider.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => Some(p.parse::<Provider>().map_err(ApiError::bad_request)?),
        _ => None,
    };
    if body.messages.is_empty() {
        return Err(ApiError::bad_request(
            "send the conversation, ending with the user's message",
        ));
    }
    let (tx, rx) = mpsc::channel::<Event>(64);
    let turn = Turn {
        writer: w,
        author,
        scope,
        out: Out { tx, stop },
        images: ai_assets::Images::default(),
    };
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    tokio::spawn(async move {
        turn.run(provider, body.model, body.messages, body.note_id)
            .await;
        flag.store(false, Ordering::SeqCst);
    });
    Ok((rx, running))
}

/// The events of `rx` as a server-sent event answer, with a heartbeat every [`HEARTBEAT`].
pub(crate) fn sse<S>(events: S) -> impl IntoResponse
where
    S: futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    let body = Sse::new(events).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat"));
    (
        [
            (CACHE_CONTROL, "no-cache"),
            (HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        body,
    )
}

/// The events a turn sends, in order, ending when it does.
pub(crate) fn receive(
    rx: mpsc::Receiver<Event>,
) -> impl futures_util::Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        let e = rx.recv().await?;
        Some((Ok::<_, Infallible>(e), rx))
    })
}

/// Where the events go. A send fails once the client has gone or the turn was stopped.
struct Out {
    tx: mpsc::Sender<Event>,
    /// Turns true when the turn is stopped (`DELETE /api/ai/chat/turns/{turn}`).
    stop: watch::Receiver<bool>,
}

impl Out {
    async fn send(&self, name: &str, data: Value) -> bool {
        if self.gone() {
            return false;
        }
        let e = Event::default().event(name).data(data.to_string());
        tokio::select! {
            sent = self.tx.send(e) => sent.is_ok(),
            _ = self.stopped() => false,
        }
    }

    fn gone(&self) -> bool {
        self.tx.is_closed() || *self.stop.borrow()
    }

    /// Resolves once the turn is stopped. With no one left to stop it, never.
    async fn stopped(&self) {
        let mut stop = self.stop.clone();
        if stop.wait_for(|s| *s).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Resolves once the client has gone or the turn is stopped.
    async fn closed(&self) {
        tokio::select! {
            _ = self.tx.closed() => {}
            _ = self.stopped() => {}
        }
    }
}

/// What a turn needs before the first call: the model and its credential, and the prompt.
struct Setup {
    provider: Provider,
    model: String,
    cred: Credential,
    system: String,
    /// The changes the tools make themselves this turn.
    direct: Direct,
    /// Brave Search, when its key is stored: `web_search` is offered only then (ai D26).
    web_search: Option<crate::brave::Search>,
    /// Whether the web tools may reach the local network (`ai_settings.fetch_private`, ai D27).
    fetch_private: bool,
    /// Which internet tools Settings > AI allows (ai D28).
    web: ai::Web,
}

/// The kinds of change the tools make themselves rather than propose (ai D22): those whose
/// confirmation Settings > AI turned off and whose write the caller's scope allows. Lacking the
/// scope, a tool proposes, whatever the switch says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Direct {
    notes: bool,
    archive: bool,
    delete: bool,
    types: bool,
    /// `download_asset` stores the file at once (ai D27).
    downloads: bool,
    /// Taxonomy changes and deletes (ai D31).
    taxonomies: bool,
}

impl Direct {
    fn of(confirm: ai::Confirm, scope: Scope) -> Direct {
        let write = scope.allows(Scope::Write);
        Direct {
            notes: !confirm.notes && write,
            archive: !confirm.archive && write,
            delete: !confirm.delete && write,
            types: !confirm.types && scope.allows(Scope::Admin),
            downloads: !confirm.downloads && write,
            taxonomies: !confirm.taxonomies && scope.allows(Scope::Admin),
        }
    }

    /// Whether the change in event `event` with data `p` is made rather than proposed.
    fn covers(self, event: &str, p: &Value) -> bool {
        match event {
            "type_proposal" => self.types,
            "taxonomy_proposal" => self.taxonomies,
            "proposal" => match p["op"].as_str().unwrap_or("update") {
                "update" | "create" => self.notes,
                "archive" | "unarchive" => self.archive,
                "delete" => self.delete,
                "download" => self.downloads,
                _ => false,
            },
            _ => false,
        }
    }
}

fn setup(
    w: &Writer,
    scope: Scope,
    provider: Option<Provider>,
    model: Option<String>,
    note_id: Option<String>,
) -> Result<Setup, AiError> {
    let state = w.state();
    let settings = Settings::load(state)?;
    let (provider, model) = settings.pick(provider, model.as_deref())?;
    let cred = Credentials::resolve(state, provider)?;
    // The prompt in use (ai D17), the default or the user's own; the open note follows it.
    let mut system = settings.chat_prompt().to_owned();
    if !system.ends_with('\n') {
        system.push('\n');
    }
    if let Some(id) = note_id
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        && let Ok(n) = w.read(&id)
    {
        let doc = doc_to_json(&n.doc);
        let title = doc["fields"]["title"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        system.push_str(&format!(
            "\n## The open note\n\nThe user has note {id} open beside this chat: type `{}`, \
             title {title:?}. \"This note\" means it; read it with get_note.\n",
            n.type_name
        ));
    }
    Ok(Setup {
        provider,
        model,
        cred,
        system,
        direct: Direct::of(settings.confirm, scope),
        web_search: crate::brave::Search::load(state),
        fetch_private: settings.fetch_private,
        web: settings.web,
    })
}

/// How one model call ended.
enum Called {
    Done {
        text: String,
        calls: Vec<ToolCall>,
        stop: Stop,
        state: Option<ProviderState>,
        usage: Usage,
    },
    Failed {
        text: String,
        error: AiError,
    },
    /// The client went away.
    Gone,
}

/// A tool's answer.
pub(crate) struct ToolOut {
    pub(crate) content: String,
    pub(crate) is_error: bool,
    pub(crate) summary: Value,
    /// An event sent before the result, its name and data: a proposal for the user to act on,
    /// or the change the tool made (ai D22).
    pub(crate) proposal: Option<(&'static str, Value)>,
}

impl ToolOut {
    pub(crate) fn error(message: impl Into<String>) -> ToolOut {
        let message = message.into();
        ToolOut {
            content: json!({ "error": message }).to_string(),
            is_error: true,
            summary: json!({ "notes": [], "message": message }),
            proposal: None,
        }
    }
}

struct Turn {
    writer: Arc<Writer>,
    author: Author,
    /// The scope the caller's credentials grant, which bounds what the tools write (ai D22).
    scope: Scope,
    out: Out,
    /// The images prepared this turn, by asset id (ai D19).
    images: ai_assets::Images,
}

impl Turn {
    async fn run(
        self,
        provider: Option<Provider>,
        model: Option<String>,
        history: Vec<Message>,
        note_id: Option<String>,
    ) {
        let w = self.writer.clone();
        let scope = self.scope;
        let setup =
            match tokio::task::spawn_blocking(move || setup(&w, scope, provider, model, note_id))
                .await
            {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    self.failed(&e, &[]).await;
                    return;
                }
                Err(e) => {
                    let e = AiError::new(ErrorKind::Unavailable, e.to_string());
                    self.failed(&e, &[]).await;
                    return;
                }
            };
        let mut tools = offered_tools(setup.web);
        if setup.web.search && setup.web_search.is_some() {
            tools.push(crate::brave::tool_spec());
        }
        let mut added: Vec<Message> = Vec::new();
        // The images read_asset has sent this turn (ai D19).
        let mut images_sent = 0usize;
        for round in 1..=MAX_ROUNDS {
            if self.out.gone() {
                return;
            }
            let mut messages = history.clone();
            messages.extend(added.iter().cloned());
            let messages = self.hydrate(messages).await;
            let req = Request {
                provider: setup.provider,
                model: setup.model.clone(),
                system: Some(setup.system.clone()),
                messages,
                tools: tools.clone(),
                schema: None,
                effort: None,
                max_tokens: None,
            };
            let began = Instant::now();
            let called = tokio::select! {
                _ = self.out.closed() => Called::Gone,
                c = self.call(&setup.cred, &req) => c,
            };
            let trail = |usage: Usage, outcome: Outcome| {
                eprintln!(
                    "{}",
                    ai::trail_line(
                        setup.provider,
                        &setup.model,
                        usage,
                        outcome,
                        began.elapsed(),
                        setup.cred.custom_host.as_deref(),
                    )
                );
            };
            let (text, calls, stop, state) = match called {
                Called::Gone => return,
                Called::Failed { text, error } => {
                    trail(error.usage, Outcome::of_error(&error));
                    let mut content = Vec::new();
                    if !text.is_empty() {
                        content.push(Content::text(text));
                    }
                    if error.kind == ErrorKind::Refusal {
                        let stop = Stop::Refusal {
                            category: error.category.clone(),
                            explanation: error.explanation.clone(),
                        };
                        self.refused(&setup, content, &stop, &mut added).await;
                    } else {
                        if !content.is_empty() {
                            content.push(Content::Notice {
                                kind: "error".into(),
                                text: "The answer stopped here.".into(),
                            });
                            added.push(assistant(content, None));
                        }
                        self.failed(&error, &added).await;
                    }
                    return;
                }
                Called::Done {
                    text,
                    calls,
                    stop,
                    state,
                    usage,
                } => {
                    trail(usage, Outcome::of_stop(&stop));
                    (text, calls, stop, state)
                }
            };
            let mut content = Vec::new();
            if !text.is_empty() {
                content.push(Content::text(text));
            }
            match &stop {
                Stop::Refusal { .. } => {
                    self.refused(&setup, content, &stop, &mut added).await;
                    return;
                }
                Stop::EndTurn | Stop::MaxTokens => {
                    if stop == Stop::MaxTokens {
                        content.push(Content::Notice {
                            kind: "cut_off".into(),
                            text: "The answer reached the output limit and is cut off.".into(),
                        });
                    }
                    added.push(assistant(content, state));
                    self.done(&stop, &added).await;
                    return;
                }
                Stop::ToolUse => {}
            }
            for c in &calls {
                content.push(Content::ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    input: c.input.clone(),
                });
            }
            added.push(assistant(content, state));
            if round == MAX_ROUNDS {
                // The model asked for another round of tools, which would need a 13th call: answer
                // its calls as not run, so the transcript stays whole, and stop.
                let mut results: Vec<Content> = calls
                    .iter()
                    .map(|c| {
                        Content::tool_result(
                            c.id.clone(),
                            format!(
                                "Not run: this question reached the limit of {MAX_ROUNDS} rounds of tool calls."
                            ),
                            true,
                        )
                    })
                    .collect();
                results.push(Content::Notice {
                    kind: "round_limit".into(),
                    text: format!(
                        "Stopped after {MAX_ROUNDS} rounds of looking things up. Ask again to go on."
                    ),
                });
                added.push(Message::tool_results(results));
                self.done(&stop, &added).await;
                return;
            }
            let mut results = Vec::with_capacity(calls.len());
            for c in calls {
                if !self
                    .out
                    .send(
                        "tool",
                        json!({ "id": c.id, "name": c.name, "input": c.input }),
                    )
                    .await
                {
                    return;
                }
                if c.name == "read_asset" && c.error.is_none() {
                    let read = self.read_asset(&c, images_sent).await;
                    if !read.parts.is_empty() {
                        images_sent += 1;
                    }
                    if !self
                        .out
                        .send(
                            "tool_result",
                            json!({ "id": c.id, "summary": read.summary, "is_error": read.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&read.content, RESULT_LIMIT),
                        is_error: read.is_error,
                        parts: read.parts,
                    });
                    continue;
                }
                // web_search (ai D26): only offered, and so only run, with a Brave key.
                if c.name == "web_search"
                    && c.error.is_none()
                    && setup.web.search
                    && let Some(search) = &setup.web_search
                {
                    let got = crate::brave::web_search(search, &c.input).await;
                    if !self
                        .out
                        .send(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: Vec::new(),
                    });
                    continue;
                }
                let web_allowed = match c.name.as_str() {
                    "fetch_url" => setup.web.read,
                    "download_asset" => setup.web.download,
                    _ => false,
                };
                if web_allowed && c.error.is_none() {
                    let got = self.web_tool(&c, &setup, images_sent).await;
                    if !got.parts.is_empty() {
                        images_sent += 1;
                    }
                    if let Some((name, p)) = got.event
                        && !self.out.send(name, p).await
                    {
                        return;
                    }
                    if !self
                        .out
                        .send(
                            "tool_result",
                            json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                        )
                        .await
                    {
                        return;
                    }
                    results.push(Content::ToolResult {
                        id: c.id,
                        content: cut(&got.content, RESULT_LIMIT),
                        is_error: got.is_error,
                        parts: got.parts,
                    });
                    continue;
                }
                let w = self.writer.clone();
                let author = self.author.clone();
                let call = c.clone();
                let direct = setup.direct;
                let got = tokio::task::spawn_blocking(move || run_tool(&w, &author, direct, &call))
                    .await
                    .unwrap_or_else(|e| ToolOut::error(e.to_string()));
                if let Some((name, p)) = got.proposal
                    && !self.out.send(name, p).await
                {
                    return;
                }
                if !self
                    .out
                    .send(
                        "tool_result",
                        json!({ "id": c.id, "summary": got.summary, "is_error": got.is_error }),
                    )
                    .await
                {
                    return;
                }
                results.push(Content::ToolResult {
                    id: c.id,
                    content: cut(&got.content, RESULT_LIMIT),
                    is_error: got.is_error,
                    parts: Vec::new(),
                });
            }
            added.push(Message::tool_results(results));
        }
    }

    /// `messages` with every image part loaded from the vault, for a provider request (ai D19).
    async fn hydrate(&self, mut messages: Vec<Message>) -> Vec<Message> {
        let has_images = messages.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, Content::ToolResult { parts, .. } if !parts.is_empty()))
        });
        if !has_images {
            return messages;
        }
        let w = self.writer.clone();
        let images = self.images.clone();
        let unloaded = messages.clone();
        tokio::task::spawn_blocking(move || {
            ai_assets::hydrate(&w, &mut messages, &images);
            messages
        })
        .await
        .unwrap_or(unloaded)
    }

    /// `read_asset`, with the turn's image count (ai D19).
    async fn read_asset(&self, call: &ToolCall, images_sent: usize) -> ai_assets::Read {
        let w = self.writer.clone();
        let images = self.images.clone();
        let input = call.input.clone();
        let may_send = images_sent < ai_assets::IMAGE_LIMIT;
        tokio::task::spawn_blocking(move || ai_assets::read_asset(&w, &input, may_send, &images))
            .await
            .unwrap_or_else(|e| ai_assets::Read {
                content: json!({ "error": e.to_string() }).to_string(),
                is_error: true,
                parts: Vec::new(),
                summary: json!({ "notes": [], "message": e.to_string() }),
            })
    }

    /// `fetch_url` or `download_asset`, with the turn's image count (ai D27).
    async fn web_tool(
        &self,
        call: &ToolCall,
        setup: &Setup,
        images_sent: usize,
    ) -> crate::ai_web::WebOut {
        let f = crate::ai_web::fetcher(setup.fetch_private);
        if call.name == "fetch_url" {
            let may_send = images_sent < ai_assets::IMAGE_LIMIT;
            crate::ai_web::fetch_url(f, &call.id, &call.input, may_send, &self.images).await
        } else {
            let direct = setup.direct.downloads;
            crate::ai_web::download_asset(self.writer.clone(), f, direct, &call.id, &call.input)
                .await
        }
    }

    /// One model call, its text and usage streamed to the client as they arrive.
    async fn call(&self, cred: &Credential, req: &Request) -> Called {
        let mut stream = match ai::stream(http(), cred, req).await {
            Ok(s) => s,
            Err(error) => {
                return Called::Failed {
                    text: String::new(),
                    error,
                };
            }
        };
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut usage = Usage::default();
        while let Some(e) = stream.next().await {
            match e {
                Err(error) => return Called::Failed { text, error },
                Ok(AiEvent::TextDelta(d)) => {
                    if !self.out.send("text", json!({ "delta": d })).await {
                        return Called::Gone;
                    }
                    text.push_str(&d);
                }
                Ok(AiEvent::ToolCall(c)) => calls.push(c),
                Ok(AiEvent::Usage(u)) => {
                    usage = u;
                    let data = json!({
                        "provider": req.provider, "model": req.model,
                        "input": u.input, "output": u.output,
                    });
                    if !self.out.send("usage", data).await {
                        return Called::Gone;
                    }
                }
                Ok(AiEvent::Done {
                    stop,
                    provider_state,
                }) => {
                    return Called::Done {
                        text,
                        calls,
                        stop,
                        state: provider_state,
                        usage,
                    };
                }
            }
        }
        Called::Failed {
            text,
            error: AiError::new(
                ErrorKind::Provider,
                format!("{} stopped answering", req.provider.label()),
            ),
        }
    }

    async fn done(&self, stop: &Stop, added: &[Message]) {
        self.out
            .send("done", json!({ "stop": stop.as_str(), "messages": added }))
            .await;
    }

    async fn failed(&self, e: &AiError, added: &[Message]) {
        self.out
            .send(
                "error",
                json!({
                    "kind": e.kind.as_str(),
                    "message": e.message,
                    "provider": e.provider,
                    "model": e.model,
                    "messages": added,
                }),
            )
            .await;
    }

    /// The provider refused (ai D11): the partial answer stays, marked cut off.
    async fn refused(
        &self,
        setup: &Setup,
        mut content: Vec<Content>,
        stop: &Stop,
        added: &mut Vec<Message>,
    ) {
        let (category, explanation) = match stop {
            Stop::Refusal {
                category,
                explanation,
            } => (category.clone(), explanation.clone()),
            _ => (None, None),
        };
        let mut notice = format!("{} {} refused", setup.provider.label(), setup.model);
        if let Some(c) = &category {
            notice.push_str(&format!(" ({c})"));
        }
        notice.push('.');
        if let Some(x) = &explanation {
            notice.push(' ');
            notice.push_str(x);
        }
        content.push(Content::Notice {
            kind: "refusal".into(),
            text: notice,
        });
        added.push(assistant(content, None));
        self.out
            .send(
                "refusal",
                json!({
                    "provider": setup.provider,
                    "model": setup.model,
                    "category": category,
                    "explanation": explanation,
                    "messages": added,
                }),
            )
            .await;
    }
}

fn assistant(content: Vec<Content>, state: Option<ProviderState>) -> Message {
    Message {
        role: Role::Assistant,
        content,
        provider_state: state,
    }
}

pub(crate) fn tool(name: &str, description: &str, properties: Value) -> ToolSpec {
    let required: Vec<&String> = properties
        .as_object()
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

/// The chat's tools: read-only ones, and proposals that write nothing unless Settings > AI turned
/// their confirmation off (ai D14, D18, D20, D22), then the web tools (ai D27).
/// The tools a turn offers: [`tool_specs`] without the internet tools Settings > AI switched
/// off (ai D28). `web_search` is added apart, only with a Brave Search key.
fn offered_tools(web: ai::Web) -> Vec<ToolSpec> {
    tool_specs()
        .into_iter()
        .filter(|t| match t.name.as_str() {
            "fetch_url" => web.read,
            "download_asset" => web.download,
            _ => true,
        })
        .collect()
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool(
            "search_notes",
            "Full-text search over every note's title and text. Returns rows of id, type, title and fields, best match first.",
            json!({
                "text": { "type": "string", "description": "The words to look for." },
                "limit": { "type": "integer", "description": "How many rows to return, 1 to 20." },
            }),
        ),
        tool(
            "query_notes",
            "Run a query in the vault query language, such as `from task where status == \"Todo\" sort due asc`. Returns at most 50 rows and the total that matched.",
            json!({ "query": { "type": "string", "description": "The query text." } }),
        ),
        tool(
            "get_note",
            "Read one note by id: its type, its version and the whole note as JSON, body included, and `assets`, the files it references (id, filename, media_type, size) for read_asset.",
            json!({ "id": { "type": "string", "description": "The note's 26-character id." } }),
        ),
        tool(
            "read_asset",
            "Read an attached file by its asset id. An image (PNG, JPEG, GIF, WebP) comes back as the image itself, scaled down when large; a PDF or text file as its text, cut at 40000 characters; anything else as its id, filename, media type and size. At most 8 images per question.",
            json!({ "id": { "type": "string", "description": "The asset's 26-character id." } }),
        ),
        tool(
            "find_assets",
            "Find stored files (assets) by name: those whose file name contains `text`, ignoring case, newest first. Empty text lists the newest. Each is its id, filename, media_type, size and created time, for read_asset or for a note's asset field.",
            json!({
                "text": { "type": "string", "description": "Part of the file name; empty for the newest assets." },
                "limit": { "type": "integer", "description": "How many assets to return, 1 to 50; 20 is usual." },
            }),
        ),
        tool(
            "backlinks",
            "The notes that link to a note, with the field or mention that links them.",
            json!({ "id": { "type": "string", "description": "The note's 26-character id." } }),
        ),
        tool(
            "list_types",
            "List every note type: its name, label, plural, description and whether it is built in.",
            json!({}),
        ),
        tool(
            "get_type",
            "Read one type: its type file's WCL text as stored, and a parsed summary of its fields (with kinds) and its templates (with their tabs and slots).",
            json!({ "name": { "type": "string", "description": "The type's name, such as `task`." } }),
        ),
        tool(
            "type_format",
            "The vocabulary of type files: their items, every field kind, templates, handlers and content blocks. Read it before proposing a type change.",
            json!({}),
        ),
        tool(
            "propose_type_change",
            "Propose a type file for the user to apply: a change to an existing type, such as its fields or templates, or a new type under a new name. It is checked as the type write would check it, and the user sees a diff of the WCL and applies or discards it. When the user lets type changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The type's name: an existing type to change, or a new name to create a type." },
                "wcl": { "type": "string", "description": "The whole new type file as WCL text, with every field and template it keeps." },
            }),
        ),
        tool(
            "list_taxonomies",
            "List every taxonomy: its name, label, description, whether it is open (notes may add terms), whether it is built in, and how many terms it holds.",
            json!({}),
        ),
        tool(
            "get_taxonomy",
            "Read one taxonomy: its taxonomy file's WCL text as stored, and every term as a path from its root, such as `Active/In progress`.",
            json!({ "name": { "type": "string", "description": "The taxonomy's name, such as `status`." } }),
        ),
        tool(
            "taxonomy_format",
            "The vocabulary of taxonomy files: the label, description, open taxonomies and nested terms, and the taxonomy field kind. Read it before proposing a taxonomy change.",
            json!({}),
        ),
        tool(
            "propose_taxonomy_change",
            "Propose a taxonomy file for the user to apply: a change to an existing taxonomy, or a new taxonomy under a new name. Renaming or moving a term needs a rename entry, so the notes holding it move with it; dropping a term a note holds is refused. It is checked as the taxonomy write would check it, and the user sees a diff of the WCL and applies or discards it. When the user lets taxonomy changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The taxonomy's name: an existing taxonomy to change, or a new name to create one." },
                "wcl": { "type": "string", "description": "The whole new taxonomy file as WCL text, with every term it keeps." },
                "renames": {
                    "type": "array",
                    "description": "Terms renamed or moved, old path to new path; empty when none are. A term's children move with it.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "from": { "type": "string", "description": "The term's path now." },
                            "to": { "type": "string", "description": "Its path in the new file." },
                        },
                        "required": ["from", "to"],
                        "additionalProperties": false,
                    },
                },
            }),
        ),
        tool(
            "propose_taxonomy_delete",
            "Propose deleting a taxonomy for the user to apply. A taxonomy a type's field uses cannot be deleted: change those types first. When the user lets taxonomy changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({ "name": { "type": "string", "description": "The taxonomy's name." } }),
        ),
        tool(
            "propose_create",
            "Propose a new note for the user to create. It is checked, and the user sees the new note and creates or discards it. When the user lets note changes go ahead without asking, it is created at once and the result says applied, with its id and version.",
            json!({
                "type": { "type": "string", "description": "The new note's type, such as `task`." },
                "note": {
                    "type": "string",
                    "description": "The whole new note as JSON text, {\"fields\": {...}, \"blocks\": [...]}.",
                },
            }),
        ),
        tool(
            "propose_archive",
            "Propose archiving a note, or taking it out of the archive, for the user to apply. When the user lets archiving go ahead without asking, it is done at once and the result says applied.",
            json!({
                "id": { "type": "string", "description": "The note's id." },
                "archived": { "type": "boolean", "description": "true to archive the note, false to take it out of the archive." },
            }),
        ),
        tool(
            "propose_delete",
            "Propose deleting a note for the user to apply. Applied, it moves the note to the trash, where the user can restore it; nothing empties the trash. When the user lets deletes go ahead without asking, the note moves to the trash at once and the result says applied.",
            json!({ "id": { "type": "string", "description": "The note's id." } }),
        ),
        tool(
            "propose_edit",
            "Propose a change to one note for the user to apply. It is checked, and the user sees a diff and applies or discards it. When the user lets note changes go ahead without asking, it is written at once and the result says applied, with the new version.",
            json!({
                "id": { "type": "string", "description": "The note's id." },
                "base_version": { "type": "string", "description": "The version get_note returned." },
                "note": {
                    "type": "string",
                    "description": "The whole new note as JSON text, {\"fields\": {...}, \"blocks\": [...]}, with every field and block it keeps.",
                },
            }),
        ),
    ]
    .into_iter()
    .chain(crate::ai_web::tool_specs())
    .collect()
}

/// `s` cut to `limit` characters, with a marker saying how much was left out.
fn cut(s: &str, limit: usize) -> String {
    let total = s.chars().count();
    if total <= limit {
        return s.to_owned();
    }
    let kept: String = s.chars().take(limit).collect();
    format!("{kept}\n[truncated: {} more characters]", total - limit)
}

/// Every string in `v` cut to `limit` characters.
fn cut_strings(v: &mut Value, limit: usize) {
    match v {
        Value::String(s) => {
            if s.chars().count() > limit {
                *s = cut(s, limit);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| cut_strings(x, limit)),
        Value::Object(o) => o.values_mut().for_each(|x| cut_strings(x, limit)),
        _ => {}
    }
}

/// The ids and titles of `rows`, for a `tool_result` summary.
fn notes_of(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .map(|r| json!({ "id": r["id"], "title": r["title"] }))
        .collect()
}

pub(crate) fn str_arg<'a>(input: &'a Value, name: &str) -> Result<&'a str, ToolOut> {
    input[name]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolOut::error(format!("{name} must be a non-empty string")))
}

fn rows_out(rows: Vec<Value>, extra: Value) -> ToolOut {
    let summary = json!({ "notes": notes_of(&rows) });
    let mut content = json!({ "rows": rows });
    if let (Some(c), Some(e)) = (content.as_object_mut(), extra.as_object()) {
        c.extend(e.clone());
    }
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary,
        proposal: None,
    }
}

/// Run one tool call. A call the model got wrong is answered as an error, not run (ai D5).
/// A proposal whose confirmation is off is applied here, in place of being sent (ai D22).
fn run_tool(w: &Writer, author: &Author, direct: Direct, call: &ToolCall) -> ToolOut {
    let out = propose_or_read(w, author, call);
    match &out.proposal {
        Some((event, p)) if direct.covers(event, p) => {
            let (event, p) = (*event, p.clone());
            make(w, author, event, &p, out.summary)
        }
        _ => out,
    }
}

/// Make the change a tool proposed, as the caller: the `applied` event, and a result telling the
/// model it was made and its new version. A failure writes nothing and goes back to the model.
fn make(w: &Writer, author: &Author, event: &str, p: &Value, summary: Value) -> ToolOut {
    let done = match apply_change(w, author, event, p) {
        Ok(done) => done,
        Err(ApplyError::Write(writer::Error::Conflict {
            id,
            current_version,
            ..
        })) => {
            let base = p["base_version"].as_str().unwrap_or_default();
            return conflict(&id, base, &current_version, "write it again");
        }
        Err(e) => {
            let message = e.to_string();
            let details = ApiError::from(e).body;
            let mut out = ToolOut::error(message.clone());
            out.content = json!({
                "error": message,
                "details": details,
                "message": "Nothing was written. Fix the problem and try again.",
            })
            .to_string();
            return out;
        }
    };
    let op = done["op"].as_str().unwrap_or_default().to_owned();
    let version = done["version"].as_str().unwrap_or_default().to_owned();
    let name = done["type_name"].as_str().unwrap_or_default().to_owned();
    let made = match op.as_str() {
        "update" => format!("The edit was written to the note. Its new version is {version}."),
        "create" => format!(
            "The note was created with id {}. Its version is {version}.",
            done["note_id"].as_str().unwrap_or_default()
        ),
        "archive" => format!("The note was archived. Its new version is {version}."),
        "unarchive" => {
            format!("The note was taken out of the archive. Its new version is {version}.")
        }
        "delete" => "The note was moved to the trash, where the user can restore it.".to_owned(),
        "type_create" => format!("The type {name} was created."),
        "taxonomy_create" | "taxonomy_change" | "taxonomy_delete" => {
            let name = done["taxonomy_name"].as_str().unwrap_or_default();
            match op.as_str() {
                "taxonomy_create" => format!("The taxonomy {name} was created."),
                "taxonomy_delete" => format!("The taxonomy {name} was deleted."),
                _ => format!(
                    "The taxonomy {name} was changed; {} notes were rewritten for its renames.",
                    done["notes"].as_u64().unwrap_or(0)
                ),
            }
        }
        _ => format!("The type {name} was changed."),
    };
    let mut content = done.clone();
    content["applied"] = json!(true);
    content["message"] = json!(format!(
        "{made} Settings > AI lets this kind of change go ahead without asking, so the user did \
         not confirm it; it is done and recorded in the history."
    ));
    let mut applied = done;
    applied["id"] = p["id"].clone();
    if event == "proposal" {
        applied["title"] = p["title"].clone();
    }
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary,
        proposal: Some(("applied", applied)),
    }
}

/// Why [`apply_change`] wrote nothing.
#[derive(Debug)]
pub enum ApplyError {
    /// The change is not one a chat proposes, or lacks a member it needs.
    Invalid(String),
    /// The type or taxonomy changed since the proposal.
    Stale(String),
    /// The write refused it: a conflict, a validation failure, a missing note.
    Write(writer::Error),
    /// A download got nothing (ai D27): the address rule, the size limit, the server.
    Fetch(ai::web::FetchError),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Invalid(m) | ApplyError::Stale(m) => f.write_str(m),
            ApplyError::Write(e) => e.fmt(f),
            ApplyError::Fetch(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<writer::Error> for ApplyError {
    fn from(e: writer::Error) -> ApplyError {
        ApplyError::Write(e)
    }
}

impl From<ApplyError> for ApiError {
    fn from(e: ApplyError) -> ApiError {
        match e {
            ApplyError::Invalid(m) => ApiError::bad_request(m),
            ApplyError::Stale(m) => ApiError::new(StatusCode::CONFLICT, "stale", m),
            ApplyError::Write(e) => e.into(),
            ApplyError::Fetch(e) => {
                use ai::web::FetchError;
                let (status, code) = match &e {
                    FetchError::LocalNetwork => (StatusCode::FORBIDDEN, "local_network"),
                    FetchError::BadUrl(_) => (StatusCode::BAD_REQUEST, "bad_request"),
                    FetchError::TooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
                    _ => (StatusCode::BAD_GATEWAY, "fetch_failed"),
                };
                ApiError::new(status, code, e.to_string())
            }
        }
    }
}

/// Apply a chat change as `author`, through the writer calls the panel's Apply reaches (ai D22).
/// `event` is `proposal`, `type_proposal` or `taxonomy_proposal`, and `p` its data as the client
/// receives it:
///
/// - `update`: the note `note_id` replaced by `after` at `base_version`;
/// - `create`: a new note of `type` from `after`;
/// - `archive` and `unarchive`: the note `note_id` at `base_version`;
/// - `delete`: the note `note_id` at `base_version`, moved to the trash, never purged (ai D21);
/// - `download`: the file at `url` downloaded and stored as a new asset called `filename`,
///   through the guarded fetcher (ai D27), answering `{op, asset_id, filename, media_type,
///   size, url}`. It waits for the download, so call this off the async threads;
/// - a type proposal: `after` as the type `name`, when the type file is still `before`; a new
///   type through the type write, a change planned and then applied as one commit;
/// - a taxonomy proposal (ai D31): when the taxonomy file is still `before`, `after` as the
///   taxonomy `name` with its `renames`, or for `op` `delete` the taxonomy deleted.
///
/// Returns `{op, note_id, version}` for a note, `{op, type_name}` for a type (`type_create` or
/// `type_change`) and `{op, taxonomy_name, notes}` for a taxonomy (`taxonomy_create`,
/// `taxonomy_change` or `taxonomy_delete`, `notes` the number rewritten). A failure writes
/// nothing.
pub fn apply_change(
    w: &Writer,
    author: &Author,
    event: &str,
    p: &Value,
) -> Result<Value, ApplyError> {
    let member = |name: &str| {
        p[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApplyError::Invalid(format!("the change has no {name}")))
    };
    if event == "type_proposal" {
        let name = member("name")?;
        let after = member("after")?;
        let before = p["before"].as_str();
        if w.read_type(name).ok().as_deref() != before {
            return Err(ApplyError::Stale(format!(
                "the type {name} changed since the proposal, so nothing was written"
            )));
        }
        if before.is_none() {
            w.put_type(name, after)?;
            return Ok(json!({ "op": "type_create", "type_name": name }));
        }
        let change = TypeChange {
            wcl: after.to_owned(),
            ..TypeChange::default()
        };
        let plan = w.plan_type_change(name, &change)?;
        w.apply_type_change(name, &change, &plan.token, author)?;
        return Ok(json!({ "op": "type_change", "type_name": name }));
    }
    if event == "taxonomy_proposal" {
        return apply_taxonomy(w, author, p);
    }
    if event != "proposal" {
        return Err(ApplyError::Invalid(format!("{event:?} is not a change")));
    }
    let op = p["op"].as_str().unwrap_or("update");
    if op == "download" {
        return crate::ai_web::apply_download(w, p);
    }
    let doc = || match &p["after"] {
        v @ Value::Object(_) => Ok(Input::Json(v.clone())),
        _ => Err(ApplyError::Invalid("the change has no after".into())),
    };
    let saved = match op {
        "update" => w.update(
            member("note_id")?,
            &doc()?,
            Some(member("base_version")?),
            author,
        )?,
        "create" => w.create(member("type")?, &doc()?, author)?,
        "archive" | "unarchive" => w.set_archived(
            member("note_id")?,
            op == "archive",
            Some(member("base_version")?),
            author,
        )?,
        "delete" => w.delete(member("note_id")?, Some(member("base_version")?), author)?,
        other => {
            return Err(ApplyError::Invalid(format!(
                "{other:?} is not a note change"
            )));
        }
    };
    Ok(json!({ "op": op, "note_id": saved.id, "version": saved.version }))
}

/// A tool call's answer, a proposal not yet applied.
fn propose_or_read(w: &Writer, author: &Author, call: &ToolCall) -> ToolOut {
    if let Some(e) = &call.error {
        return ToolOut::error(e.clone());
    }
    let input = &call.input;
    let result = match call.name.as_str() {
        "search_notes" => str_arg(input, "text").and_then(|text| {
            let limit = input["limit"]
                .as_u64()
                .map_or(10, |n| usize::try_from(n).unwrap_or(SEARCH_LIMIT))
                .clamp(1, SEARCH_LIMIT);
            Vault::search(w, text, limit, script_core::Archived::Exclude)
                .map(|rows| rows_out(rows, json!({})))
                .map_err(ToolOut::error)
        }),
        "query_notes" => str_arg(input, "query").and_then(|q| {
            Vault::query_page(w, q, QUERY_LIMIT, None)
                .map(|page| {
                    let rows = page["rows"].as_array().cloned().unwrap_or_default();
                    rows_out(
                        rows,
                        json!({ "total": page["total"], "more": !page["next"].is_null() }),
                    )
                })
                .map_err(ToolOut::error)
        }),
        "get_note" => str_arg(input, "id").and_then(|id| {
            Vault::get(w, id)
                .map(|mut note| {
                    cut_strings(&mut note, BODY_LIMIT);
                    note["assets"] = json!(ai_assets::note_assets(w, &note["note"]));
                    let summary = json!({ "notes": [{
                        "id": note["id"], "title": note["note"]["fields"]["title"],
                    }] });
                    ToolOut {
                        content: note.to_string(),
                        is_error: false,
                        summary,
                        proposal: None,
                    }
                })
                .map_err(ToolOut::error)
        }),
        "find_assets" => {
            let found = ai_assets::find_assets(w, input);
            Ok(ToolOut {
                content: found.content,
                is_error: found.is_error,
                summary: found.summary,
                proposal: None,
            })
        }
        "backlinks" => str_arg(input, "id").and_then(|id| {
            Vault::backlinks(w, id)
                .map(|rows| rows_out(rows, json!({})))
                .map_err(ToolOut::error)
        }),
        "propose_edit" => propose(w, author, &call.id, input),
        "propose_create" => propose_create(w, author, &call.id, input),
        "propose_archive" => propose_archive(w, &call.id, input),
        "propose_delete" => propose_delete(w, author, &call.id, input),
        "list_types" => Ok(list_types(w)),
        "get_type" => get_type(w, input),
        "type_format" => Ok(type_format()),
        "propose_type_change" => propose_type(w, &call.id, input),
        "list_taxonomies" => Ok(crate::ai_taxonomies::list(w)),
        "get_taxonomy" => crate::ai_taxonomies::get(w, input),
        "taxonomy_format" => Ok(crate::ai_taxonomies::format()),
        "propose_taxonomy_change" => crate::ai_taxonomies::propose_change(w, &call.id, input),
        "propose_taxonomy_delete" => crate::ai_taxonomies::propose_delete(w, &call.id, input),
        other => Err(ToolOut::error(format!("there is no tool {other:?}"))),
    };
    result.unwrap_or_else(|e| e)
}

/// `propose_edit`: check the edit with a dry-run batch and hand it to the user (ai D14).
fn propose(w: &Writer, author: &Author, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let base = str_arg(input, "base_version")?;
    let note = note_arg(input)?;
    let current = w.read(id).map_err(|e| ToolOut::error(e.to_string()))?;
    let before = doc_to_json(&current.doc);
    let op = Op::Update {
        id: id.to_owned(),
        input: Input::Json(note.clone()),
        base: Some(base.to_owned()),
    };
    let report = w
        .batch(vec![op], true, author)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    if let Some(e) = report.error {
        return Err(ToolOut::error(e.to_string()));
    }
    let item = match report.items.into_iter().next() {
        Some(Ok(item)) => item,
        Some(Err(writer::Error::Conflict {
            current_version, ..
        })) => {
            return Err(conflict(
                id,
                base,
                &current_version,
                "propose against that version",
            ));
        }
        Some(Err(e)) => return Err(item_error(e)),
        None => return Err(ToolOut::error("the edit was not checked")),
    };
    let after = item
        .wcl
        .as_deref()
        .and_then(|t| Input::Wcl(t.to_owned()).parse("note.wcl").ok())
        .map(|d| doc_to_json(&d))
        .unwrap_or(note);
    let ops = diff(&before, &after);
    let title = after["fields"]["title"].clone();
    let summary = json!({ "notes": [{ "id": id, "title": title }] });
    if ops.is_empty() {
        return Ok(ToolOut {
            content: json!({
                "proposed": false,
                "message": "The proposed note is the same as the current one: there is nothing to apply.",
            })
            .to_string(),
            is_error: false,
            summary,
            proposal: None,
        });
    }
    let proposal = json!({
        "id": call_id,
        "op": "update",
        "note_id": id,
        "title": title,
        "base_version": base,
        "before": before,
        "after": after,
        "ops": ops,
    });
    Ok(ToolOut {
        content: json!({
            "proposed": true,
            "note_id": id,
            "changes": ops.len(),
            "message": "The edit is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it.",
        })
        .to_string(),
        is_error: false,
        summary,
        proposal: Some(("proposal", proposal)),
    })
}

/// A note that changed since the version the model based its change on.
fn conflict(id: &str, base: &str, current_version: &str, then: &str) -> ToolOut {
    ToolOut::error(format!(
        "conflict: note {id} changed since version {base}; its current version is \
         {current_version}. Nothing was written. Read it again with get_note and {then}."
    ))
}

/// The `note` argument: the whole note as JSON text, or as an object.
fn note_arg(input: &Value) -> Result<Value, ToolOut> {
    match &input["note"] {
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| ToolOut::error(format!("note is not JSON: {e}")))
        }
        v @ Value::Object(_) => Ok(v.clone()),
        _ => Err(ToolOut::error("note must be the whole note as JSON text")),
    }
}

/// A batch item's failure for the model: the message, and the API's error body with the
/// validation issues.
fn item_error(e: writer::Error) -> ToolOut {
    let message = e.to_string();
    let details = ApiError::from(e).body;
    let mut out = ToolOut::error(message.clone());
    out.content = json!({
        "error": message,
        "details": details,
        "message": "Nothing was proposed. Fix the problem and propose it again.",
    })
    .to_string();
    out
}

/// One operation as a dry-run batch: what it would do, or its error for the model.
fn dry_run(w: &Writer, author: &Author, op: Op) -> Result<writer::Item, ToolOut> {
    let report = w
        .batch(vec![op], true, author)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    if let Some(e) = report.error {
        return Err(item_error(e));
    }
    match report.items.into_iter().next() {
        Some(Ok(item)) => Ok(item),
        Some(Err(e)) => Err(item_error(e)),
        None => Err(ToolOut::error("the change was not checked")),
    }
}

/// A note proposal's answer: the `proposal` event, and `content` telling the model it awaits
/// the user.
fn proposed(content: Value, summary: Value, proposal: Value) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary,
        proposal: Some(("proposal", proposal)),
    }
}

/// `propose_create`: check a new note with a dry-run batch create and hand it to the user
/// (ai D20).
fn propose_create(
    w: &Writer,
    author: &Author,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let type_name = str_arg(input, "type")?;
    let note = note_arg(input)?;
    let op = Op::Create {
        type_name: type_name.to_owned(),
        input: Input::Json(note.clone()),
    };
    let item = dry_run(w, author, op)?;
    let after = item
        .wcl
        .as_deref()
        .and_then(|t| Input::Wcl(t.to_owned()).parse("note.wcl").ok())
        .map(|d| doc_to_json(&d))
        .unwrap_or(note);
    let title = after["fields"]["title"].clone();
    let proposal = json!({
        "id": call_id,
        "op": "create",
        "type": type_name,
        "title": title,
        "after": after,
    });
    Ok(proposed(
        json!({
            "proposed": true,
            "op": "create",
            "type": type_name,
            "message": "The new note is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who creates or discards it.",
        }),
        json!({ "notes": [] }),
        proposal,
    ))
}

/// `propose_archive`: check the note exists and is not already in that state, and hand the
/// archive or unarchive to the user (ai D20).
fn propose_archive(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let archived = input["archived"]
        .as_bool()
        .ok_or_else(|| ToolOut::error("archived must be true or false"))?;
    let note = w.read(id).map_err(|e| ToolOut::error(e.to_string()))?;
    if writer::is_archived(&note.doc) == archived {
        return Err(ToolOut::error(if archived {
            format!("note {id} is already archived: there is nothing to apply")
        } else {
            format!("note {id} is not archived: there is nothing to apply")
        }));
    }
    let title = doc_to_json(&note.doc)["fields"]["title"].clone();
    let op = if archived { "archive" } else { "unarchive" };
    let proposal = json!({
        "id": call_id,
        "op": op,
        "note_id": id,
        "title": title,
        "base_version": note.version,
    });
    Ok(proposed(
        json!({
            "proposed": true,
            "op": op,
            "note_id": id,
            "message": format!(
                "The {op} is shown to the user as a proposal. It has not been done: it awaits \
                 the user, who applies or discards it."
            ),
        }),
        json!({ "notes": [{ "id": id, "title": title }] }),
        proposal,
    ))
}

/// `propose_delete`: check the note can be deleted with a dry-run batch delete, and hand the
/// delete to the user (ai D20). Applied, it moves the note to the trash; nothing here purges.
fn propose_delete(
    w: &Writer,
    author: &Author,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let note = w.read(id).map_err(|e| ToolOut::error(e.to_string()))?;
    let op = Op::Delete {
        id: id.to_owned(),
        base: Some(note.version.clone()),
    };
    dry_run(w, author, op)?;
    let title = doc_to_json(&note.doc)["fields"]["title"].clone();
    let proposal = json!({
        "id": call_id,
        "op": "delete",
        "note_id": id,
        "title": title,
        "base_version": note.version,
    });
    Ok(proposed(
        json!({
            "proposed": true,
            "op": "delete",
            "note_id": id,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it. Applied, the note moves to the trash, where the user can restore it.",
        }),
        json!({ "notes": [{ "id": id, "title": title }] }),
        proposal,
    ))
}

/// A type tool's answer: `content` for the model, and the type names for the summary.
fn type_out(content: Value, types: Vec<Value>, type_proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [], "types": types }),
        proposal: type_proposal.map(|p| ("type_proposal", p)),
    }
}

/// `list_types`: every type's name, label, plural, description and whether it is built in.
fn list_types(w: &Writer) -> ToolOut {
    let schema = w.schema();
    let mut types: Vec<&schema::TypeDef> = schema.types().collect();
    types.sort_by(|a, b| a.name.cmp(&b.name));
    let rows: Vec<Value> = types
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "label": t.label,
                "plural": t.plural,
                "description": t.description,
                "builtin": w.is_built_in(&t.name),
            })
        })
        .collect();
    let names = types.iter().map(|t| json!(t.name)).collect();
    type_out(json!({ "types": rows }), names, None)
}

/// `get_type`: the type file as stored, and its fields and templates as the types API lists them.
fn get_type(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = w
        .read_type(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let schema = w.schema();
    let summary = schema
        .type_def(name)
        .map(|t| crate::types::type_json(t, w.is_built_in(name)))
        .unwrap_or(Value::Null);
    Ok(type_out(
        json!({ "name": name, "wcl": wcl, "summary": summary }),
        vec![json!(name)],
        None,
    ))
}

/// The vault format document, embedded at build time.
pub(crate) const VAULT_FORMAT: &str = include_str!("../../../docs/vault-format.md");

/// The sections of `docs/vault-format.md` a type file is written from: type files to handlers,
/// and the content blocks a template's tabs hold.
pub fn type_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        let section = |from: &str, to: &str| {
            let start = VAULT_FORMAT.find(from).unwrap_or(VAULT_FORMAT.len());
            let end = VAULT_FORMAT[start..]
                .find(to)
                .map_or(VAULT_FORMAT.len(), |e| start + e);
            VAULT_FORMAT[start..end].trim().to_owned()
        };
        format!(
            "{}\n\n{}\n",
            section("\n## Type files\n", "\n## Note files\n"),
            section("\n## Content blocks\n", "\n## Inline markup\n"),
        )
    })
}

/// `type_format`: the type-file vocabulary.
fn type_format() -> ToolOut {
    type_out(json!({ "format": type_format_text() }), Vec::new(), None)
}

/// `propose_type_change`: check the type file as the type write would, without writing it, and
/// hand it to the user (ai D18). Invalid WCL goes back to the model with the issues.
fn propose_type(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = input["wcl"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolOut::error("wcl must be the whole type file as WCL text"))?;
    let before = w.read_type(name).ok();
    let check = w.check_type(name, wcl).map_err(|e| {
        let message = e.to_string();
        let details = ApiError::from(e).body;
        let mut out = ToolOut::error(message.clone());
        out.content = json!({
            "error": message,
            "issues": details,
            "message": "The type was not proposed. Fix the issues and propose it again.",
        })
        .to_string();
        out
    })?;
    let issues: Vec<Value> = check
        .warnings
        .iter()
        .map(crate::error::issue_json)
        .collect();
    let names = vec![json!(name)];
    if before.as_deref() == Some(check.wcl.as_str()) {
        return Ok(type_out(
            json!({
                "proposed": false,
                "message": "The proposed type file is the same as the current one: there is nothing to apply.",
            }),
            names,
            None,
        ));
    }
    let mut content = json!({
        "proposed": true,
        "name": name,
        "exists": check.exists,
        "warnings": issues,
        "message": if check.exists {
            "The type change is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it."
        } else {
            "The new type is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who applies or discards it."
        },
    });
    if let Some(plan) = &check.plan {
        content["plan"] = json!({
            "added": plan.added,
            "removed": plan.removed,
            "notes_edited": plan.changes.len(),
            "notes_flagged": plan.flagged.len(),
        });
    }
    let proposal = json!({
        "id": call_id,
        "name": name,
        "exists": check.exists,
        "before": before,
        "after": check.wcl,
        "issues": issues,
    });
    Ok(type_out(content, names, Some(proposal)))
}

/// What a proposal changes: each field whose value differs, then each top-level block (matched
/// by kind and labels) added, removed or changed. Server-set fields are left out.
pub fn diff(before: &Value, after: &Value) -> Vec<Value> {
    let mut ops = Vec::new();
    let empty = serde_json::Map::new();
    let bf = before["fields"].as_object().unwrap_or(&empty);
    let af = after["fields"].as_object().unwrap_or(&empty);
    let mut names: Vec<&String> = bf.keys().chain(af.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        if SERVER_FIELDS.contains(&name.as_str()) {
            continue;
        }
        let (b, a) = (bf.get(name), af.get(name));
        if b != a {
            ops.push(json!({
                "op": "field", "name": name,
                "before": b.cloned().unwrap_or(Value::Null),
                "after": a.cloned().unwrap_or(Value::Null),
            }));
        }
    }
    let key = |b: &Value| {
        let labels: Vec<&str> = b["labels"]
            .as_array()
            .map(|l| l.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let kind = b["kind"].as_str().unwrap_or_default();
        if labels.is_empty() {
            kind.to_owned()
        } else {
            format!("{kind} {}", labels.join(" "))
        }
    };
    let no_blocks = Vec::new();
    let bb = before["blocks"].as_array().unwrap_or(&no_blocks);
    let ab = after["blocks"].as_array().unwrap_or(&no_blocks);
    let mut keys: Vec<String> = Vec::new();
    for b in bb.iter().chain(ab.iter()) {
        let k = key(b);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    for k in keys {
        let b: Vec<&Value> = bb.iter().filter(|x| key(x) == k).collect();
        let a: Vec<&Value> = ab.iter().filter(|x| key(x) == k).collect();
        if b != a {
            ops.push(json!({
                "op": "block", "name": k,
                "before": if b.is_empty() { Value::Null } else if b.len() == 1 { b[0].clone() } else { json!(b) },
                "after": if a.is_empty() { Value::Null } else if a.len() == 1 { a[0].clone() } else { json!(a) },
            }));
        }
    }
    ops
}

/// `GET /api/ai/models` and the proposal diff are tested through the server; the pure helpers
/// here.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_marks_what_it_leaves_out() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abcdef", 4), "abcd\n[truncated: 2 more characters]");
    }

    #[test]
    fn diff_lists_changed_fields_and_blocks_but_not_server_fields() {
        let before = json!({
            "fields": { "title": "A", "updated": "x", "tags": ["a"] },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [] }],
        });
        let after = json!({
            "fields": { "title": "B", "updated": "y", "tags": ["a"] },
            "blocks": [{ "kind": "content", "labels": ["body"], "blocks": [{ "kind": "paragraph" }] }],
        });
        let ops = diff(&before, &after);
        assert_eq!(ops.len(), 2, "{ops:?}");
        assert_eq!(ops[0]["name"], "title");
        assert_eq!(ops[1]["name"], "content body");
    }

    #[test]
    fn type_format_holds_the_type_file_sections() {
        let t = type_format_text();
        assert!(t.starts_with("## Type files"), "{}", &t[..40]);
        assert!(t.contains("## Templates"));
        assert!(t.contains("## Content blocks"));
        assert!(!t.contains("## Note files"));
        assert!(t.len() < RESULT_LIMIT);
    }

    #[test]
    fn tools_are_strict_objects() {
        for t in tool_specs() {
            let s = &t.input_schema;
            assert_eq!(s["additionalProperties"], false);
            let props = s["properties"].as_object().unwrap().len();
            assert_eq!(s["required"].as_array().unwrap().len(), props, "{}", t.name);
        }
    }
}
