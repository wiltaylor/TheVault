//! `POST /api/ai/chat`: the chat panel's conversation turn (ai D10, D11, D14).
//!
//! The body is `{ provider?, model?, messages, note_id? }`: the transcript so far, ending with the
//! user's new message, in the neutral form of [`ai::Message`]. Provider and model default to
//! Settings > AI's. `note_id` names the note open beside the panel: its id, type and title (never
//! its body) join the system prompt, so "this note" resolves through `get_note`.
//!
//! The answer is a server-sent event stream. The server runs the tool loop, at most
//! [`MAX_ROUNDS`] model calls per user turn, over read-only tools; `propose_edit` checks an edit
//! with a dry-run batch and never writes it. Events:
//!
//! | Event | Data |
//! |---|---|
//! | `text` | `{delta}`: answer text as it arrives |
//! | `tool` | `{id, name, input}`: a tool the model called, before it runs |
//! | `tool_result` | `{id, summary, is_error}`: `summary` holds the ids and titles returned (`notes`), never bodies, and a `message` on an error |
//! | `proposal` | `{id, note_id, title, base_version, before, after, ops}`: an edit for the user to apply |
//! | `usage` | `{provider, model, input, output}`: one model call's tokens |
//! | `refusal` | `{provider, model, category, explanation, messages}`: the provider refused; ends the stream |
//! | `error` | `{kind, message, messages}`: the turn failed; ends the stream |
//! | `done` | `{stop, messages}`: the turn is over |
//!
//! `messages` is the turn's new transcript items, for the client to append and send back next
//! turn: assistant messages with their `provider_state`, and the user messages carrying tool
//! results. They are never edited afterwards. A comment goes out every [`HEARTBEAT`]. When the
//! client goes away the loop stops, and the provider call in flight is dropped, which closes its
//! connection.

use std::convert::Infallible;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ai::{
    AiError, Content, Credential, Credentials, ErrorKind, Event as AiEvent, Message, Outcome,
    Provider, ProviderState, Request, Role, Settings, Stop, ToolCall, ToolSpec, Usage,
};
use axum::Json;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, HeaderName};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use script_core::Vault;
use serde::Deserialize;
use serde_json::{Value, json};
use state::Author;
use tokio::sync::mpsc;
use writer::{Input, Op, Writer, doc_to_json};

use crate::ApiError;
use crate::auth::Caller;
use crate::events::HEARTBEAT;

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
    Json(body): Json<ChatBody>,
) -> Result<impl IntoResponse, ApiError> {
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
        out: Out { tx },
    };
    tokio::spawn(turn.run(provider, body.model, body.messages, body.note_id));
    let events = futures_util::stream::unfold(rx, |mut rx| async move {
        let e = rx.recv().await?;
        Some((Ok::<_, Infallible>(e), rx))
    });
    let body = Sse::new(events).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat"));
    Ok((
        [
            (CACHE_CONTROL, "no-cache"),
            (HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        body,
    ))
}

/// Where the events go. A send fails once the client has gone.
struct Out {
    tx: mpsc::Sender<Event>,
}

impl Out {
    async fn send(&self, name: &str, data: Value) -> bool {
        let e = Event::default().event(name).data(data.to_string());
        self.tx.send(e).await.is_ok()
    }

    fn gone(&self) -> bool {
        self.tx.is_closed()
    }
}

/// What a turn needs before the first call: the model and its credential, and the prompt.
struct Setup {
    provider: Provider,
    model: String,
    cred: Credential,
    system: String,
}

fn setup(
    w: &Writer,
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
struct ToolOut {
    content: String,
    is_error: bool,
    summary: Value,
    proposal: Option<Value>,
}

impl ToolOut {
    fn error(message: impl Into<String>) -> ToolOut {
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
    out: Out,
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
        let setup =
            match tokio::task::spawn_blocking(move || setup(&w, provider, model, note_id)).await {
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
        let tools = tool_specs();
        let mut added: Vec<Message> = Vec::new();
        for round in 1..=MAX_ROUNDS {
            if self.out.gone() {
                return;
            }
            let mut messages = history.clone();
            messages.extend(added.iter().cloned());
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
                _ = self.out.tx.closed() => Called::Gone,
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
                        began.elapsed()
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
                    .map(|c| Content::ToolResult {
                        id: c.id.clone(),
                        content: format!(
                            "Not run: this question reached the limit of {MAX_ROUNDS} rounds of tool calls."
                        ),
                        is_error: true,
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
                let w = self.writer.clone();
                let author = self.author.clone();
                let call = c.clone();
                let got = tokio::task::spawn_blocking(move || run_tool(&w, &author, &call))
                    .await
                    .unwrap_or_else(|e| ToolOut::error(e.to_string()));
                if let Some(p) = got.proposal
                    && !self.out.send("proposal", p).await
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
                });
            }
            added.push(Message::tool_results(results));
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

fn tool(name: &str, description: &str, properties: Value) -> ToolSpec {
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

/// The chat's tools, all read-only (ai D14).
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
            "Read one note by id: its type, its version and the whole note as JSON, body included.",
            json!({ "id": { "type": "string", "description": "The note's 26-character id." } }),
        ),
        tool(
            "backlinks",
            "The notes that link to a note, with the field or mention that links them.",
            json!({ "id": { "type": "string", "description": "The note's 26-character id." } }),
        ),
        tool(
            "propose_edit",
            "Propose a change to one note for the user to apply. It is checked but never written: the user sees a diff and applies or discards it.",
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

fn str_arg<'a>(input: &'a Value, name: &str) -> Result<&'a str, ToolOut> {
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
fn run_tool(w: &Writer, author: &Author, call: &ToolCall) -> ToolOut {
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
        "backlinks" => str_arg(input, "id").and_then(|id| {
            Vault::backlinks(w, id)
                .map(|rows| rows_out(rows, json!({})))
                .map_err(ToolOut::error)
        }),
        "propose_edit" => propose(w, author, &call.id, input),
        other => Err(ToolOut::error(format!("there is no tool {other:?}"))),
    };
    result.unwrap_or_else(|e| e)
}

/// `propose_edit`: check the edit with a dry-run batch and hand it to the user (ai D14).
fn propose(w: &Writer, author: &Author, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let id = str_arg(input, "id")?;
    let base = str_arg(input, "base_version")?;
    let note: Value = match &input["note"] {
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| ToolOut::error(format!("note is not JSON: {e}")))?
        }
        v @ Value::Object(_) => v.clone(),
        _ => return Err(ToolOut::error("note must be the whole note as JSON text")),
    };
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
            return Err(ToolOut::error(format!(
                "conflict: note {id} changed since version {base}; its current version is \
                 {current_version}. Read it again with get_note and propose against that version."
            )));
        }
        Some(Err(e)) => {
            let message = e.to_string();
            let details = ApiError::from(e).body;
            let mut out = ToolOut::error(message.clone());
            out.content = json!({ "error": message, "details": details }).to_string();
            return Err(out);
        }
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
        proposal: Some(proposal),
    })
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
    fn tools_are_strict_objects() {
        for t in tool_specs() {
            let s = &t.input_schema;
            assert_eq!(s["additionalProperties"], false);
            let props = s["properties"].as_object().unwrap().len();
            assert_eq!(s["required"].as_array().unwrap().len(), props, "{}", t.name);
        }
    }
}
