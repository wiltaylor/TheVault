//! Agent threads (agents G7, G8): each conversation with an agent is a note of the built-in type
//! `agent_thread`.
//!
//! A thread note holds `title`, `agent` (the agent's name), `origin` (`user`, or
//! `trigger:<name>`), `last_activity`, the hidden `pending` (how many proposals wait for the
//! user, for the Chat tab's badge) and the hidden `transcript`, the JSON of [`Transcript`]: the
//! D10 messages with their `provider_state`, so the thread resumes exactly, each tool step's
//! summary, and every proposal the agent made with its outcome. The content block `body` holds
//! the conversation as readable text, rewritten with each write, so threads are searchable.
//! Every write the server makes to a thread is authored `Author::Agent(<agent>)` (agents G11).
//!
//! Routes:
//!
//! - `GET /api/agents/{name}/threads` (read): the agent's threads, newest activity first, each
//!   `{id, title, origin, last_activity, pending, archived, running, turn, started_ms}`:
//!   `running` says a turn runs on the thread now, `turn` is its id in the turns form (null for a
//!   trigger's run) and `started_ms` when it started (agents G17).
//! - `POST /api/agents/{name}/threads` (read) takes `{title?}` and creates a thread, 201 with the
//!   thread.
//! - `GET /api/agents/{name}/threads/{id}` (read): the thread with its `messages`, `steps` and
//!   `proposals`, and `running`, `turn` and `started_ms` as the list has them.
//! - `PATCH /api/agents/{name}/threads/{id}` (write) takes `{title}`: a rename, authored by the
//!   caller.
//! - `DELETE /api/agents/{name}/threads/{id}` (write): the note goes to the trash.
//!
//! The chat routes take `agent` and `thread` (`crate::ai_chat`): the server reads the thread's
//! transcript itself and writes each round to it as the turn streams. `POST /api/ai/apply` with
//! a proposal's `thread` records its outcome here, and `POST /api/ai/discard` records a discard;
//! the next turn on the thread starts with the D29 lines for every outcome not yet reported.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use ai::{Content, Message, Role};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use state::Author;
use writer::{Input, Writer, doc_to_json};

use crate::auth::Caller;
use crate::{ApiError, blocking};

/// The note type of a thread.
pub const THREAD_TYPE: &str = writer::AGENT_THREAD_TYPE;

/// The title of a thread nobody named, until its first message names it.
pub const DEFAULT_TITLE: &str = "New thread";

/// The longest title the first message gives a thread, in characters.
const TITLE_CHARS: usize = 60;

/// How often a write to a thread that changed meanwhile is tried again.
const RETRIES: usize = 5;

/// What a thread's hidden `transcript` field holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// The D10 messages, as the provider requests replay them.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Each tool step's `tool_result` summary and error flag, by tool call id, so a reopened
    /// thread shows the notes a step found.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub steps: Map<String, Value>,
    /// Every proposal the agent made, and the changes it made itself, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<Stored>,
}

/// One proposal of a thread and its outcome (agents G8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    /// `proposal`, `type_proposal`, `taxonomy_proposal`, `agent_proposal`, or `applied` for a change the agent
    /// made itself.
    pub event: String,
    /// The event's data as the client received it.
    pub data: Value,
    pub status: Status,
    /// What `POST /api/ai/apply` answered, once applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Why it failed or went stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Whether the model has been told of the outcome (ai D29).
    #[serde(default, skip_serializing_if = "is_false")]
    pub reported: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Where a proposal stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting for the user.
    Pending,
    Applied,
    Discarded,
    /// The apply failed; the user may try again.
    Failed,
    /// The note, type or taxonomy changed since the proposal, so it can never apply.
    Stale,
    /// A change the agent made itself, its confirmation off (ai D22).
    Made,
}

impl Stored {
    /// The proposal's id, its tool call's id.
    pub fn id(&self) -> &str {
        self.data["id"].as_str().unwrap_or_default()
    }

    /// Whether it still waits for the user: pending, or failed and retryable.
    pub fn waits(&self) -> bool {
        matches!(self.status, Status::Pending | Status::Failed)
    }
}

impl Transcript {
    /// The transcript a thread's `transcript` field holds; an empty or unreadable one is empty.
    pub fn parse(text: &str) -> Transcript {
        if text.trim().is_empty() {
            return Transcript::default();
        }
        serde_json::from_str(text).unwrap_or_else(|e| {
            eprintln!("a thread transcript does not read, so the thread starts again: {e}");
            Transcript::default()
        })
    }

    /// How many proposals wait for the user.
    pub fn pending(&self) -> usize {
        self.proposals.iter().filter(|p| p.waits()).count()
    }

    /// The D29 lines for every outcome the model has not been told of, each marked told.
    pub fn take_outcome_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        for p in &mut self.proposals {
            if p.reported || matches!(p.status, Status::Pending | Status::Made) {
                continue;
            }
            if let Some(line) = outcome_line(p) {
                lines.push(line);
            }
            p.reported = true;
        }
        lines
    }
}

/// `"<title>"` with a space before it, or nothing for no title.
fn quoted(title: &Value) -> String {
    match title.as_str() {
        Some(t) if !t.is_empty() => format!(" {t:?}"),
        _ => String::new(),
    }
}

/// What a proposal is, for an outcome line or the readable body.
pub fn proposal_what(event: &str, d: &Value) -> String {
    let s = |k: &str| d[k].as_str().unwrap_or_default().to_owned();
    match event {
        "type_proposal" => {
            if d["exists"] == true {
                format!("change to type {}", s("name"))
            } else {
                format!("new type {}", s("name"))
            }
        }
        "taxonomy_proposal" => match d["op"].as_str() {
            Some("delete") => format!("delete of taxonomy {}", s("name")),
            _ if d["exists"] == true => format!("change to taxonomy {}", s("name")),
            _ => format!("new taxonomy {}", s("name")),
        },
        "agent_proposal" => match d["op"].as_str() {
            Some("delete") => format!("delete of agent {}", s("name")),
            _ if d["exists"] == true => format!("change to agent {}", s("name")),
            _ => format!("new agent {}", s("name")),
        },
        "agent_memory_proposal" => match d["op"].as_str() {
            Some("delete") => format!(
                "forgetting memory {} of agent {}",
                s("memory_id"),
                s("memory_agent")
            ),
            Some("change") => format!(
                "change to memory {} of agent {}",
                s("memory_id"),
                s("memory_agent")
            ),
            _ => format!(
                "new memory{} of agent {}",
                quoted(&d["title"]),
                s("memory_agent")
            ),
        },
        _ => match d["op"].as_str().unwrap_or("update") {
            "update" => format!("edit of note {}", s("note_id")),
            "create" => format!("new {} note{}", s("type"), quoted(&d["title"])),
            "download" if d["source"] == "browser" => {
                format!("save of {} from the browser box", s("path"))
            }
            "download" => format!("download of {}", s("url")),
            op => format!("{op} of note {}", s("note_id")),
        },
    }
}

/// What an applied proposal did, from `done`, `POST /api/ai/apply`'s answer.
fn proposal_done(event: &str, d: &Value, done: &Value) -> String {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    let version = s(done, "version");
    match event {
        "type_proposal" => {
            if d["exists"] == true {
                format!("change to type {}", s(d, "name"))
            } else {
                format!("created type {}", s(d, "name"))
            }
        }
        "taxonomy_proposal" => match d["op"].as_str() {
            Some("delete") => format!("deleted taxonomy {}", s(d, "name")),
            _ if d["exists"] != true => format!("created taxonomy {}", s(d, "name")),
            _ => {
                let notes = done["notes"].as_u64().unwrap_or(0);
                let moved = match notes {
                    0 => String::new(),
                    1 => ", its renames moving 1 note".to_owned(),
                    n => format!(", its renames moving {n} notes"),
                };
                format!("changed taxonomy {}{moved}", s(d, "name"))
            }
        },
        "agent_proposal" => match d["op"].as_str() {
            Some("delete") => format!("deleted agent {}", s(d, "name")),
            _ if d["exists"] == true => format!("changed agent {}", s(d, "name")),
            _ => format!("created agent {}", s(d, "name")),
        },
        "agent_memory_proposal" => match d["op"].as_str() {
            Some("delete") => format!(
                "forgot memory {} of agent {}: moved to the trash",
                s(d, "memory_id"),
                s(d, "memory_agent")
            ),
            Some("change") => format!(
                "changed memory {} of agent {}",
                s(d, "memory_id"),
                s(d, "memory_agent")
            ),
            _ => format!(
                "created memory {}{} of agent {}",
                s(done, "memory_id"),
                quoted(&d["title"]),
                s(d, "memory_agent")
            ),
        },
        _ => match d["op"].as_str().unwrap_or("update") {
            "create" => {
                let v = if version.is_empty() {
                    String::new()
                } else {
                    format!(", version {version}")
                };
                format!(
                    "created note {}{}{v}",
                    s(done, "note_id"),
                    quoted(&d["title"])
                )
            }
            "delete" => format!("delete of note {}: moved to the trash", s(d, "note_id")),
            "download" => format!(
                "{} stored as asset {} ({})",
                if d["source"] == "browser" {
                    format!("save of {} from the browser box", s(d, "path"))
                } else {
                    format!("download of {}", s(d, "url"))
                },
                s(done, "asset_id"),
                s(done, "filename")
            ),
            _ => {
                let what = proposal_what(event, d);
                if version.is_empty() {
                    what
                } else {
                    format!("{what} now at version {version}")
                }
            }
        },
    }
}

/// The D29 line for an outcome, or none for a pending proposal or a change the agent made.
pub fn outcome_line(p: &Stored) -> Option<String> {
    let what = proposal_what(&p.event, &p.data);
    match p.status {
        Status::Applied => Some(format!(
            "[Applied by the user: {}]",
            proposal_done(&p.event, &p.data, p.result.as_ref().unwrap_or(&Value::Null))
        )),
        Status::Discarded => Some(format!("[Discarded by the user: {what}]")),
        Status::Failed | Status::Stale => Some(format!(
            "[Failed to apply: {what}: {}]",
            p.error.as_deref().unwrap_or("it did not apply")
        )),
        Status::Pending | Status::Made => None,
    }
}

/// Whether `text` is a block of D29 outcome lines rather than the user's own words.
pub fn is_outcome_lines(text: &str) -> bool {
    let t = text.trim_start();
    [
        "[Applied by the user:",
        "[Discarded by the user:",
        "[Failed to apply:",
    ]
    .iter()
    .any(|p| t.starts_with(p))
}

/// One thread note, read.
#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub version: String,
    pub title: String,
    pub agent: String,
    pub origin: String,
    pub last_activity: Option<String>,
    /// Whether the thread note is archived (agents G15).
    pub archived: bool,
    pub transcript: Transcript,
    /// The whole note in its JSON form, so a write keeps what it does not set.
    doc: Value,
}

fn not_a_thread(id: &str) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("there is no agent thread {id}"),
    )
}

/// The time now, as a `datetime` field holds it.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

impl Thread {
    /// Thread note `id`. A note of another type is a 404.
    pub fn read(w: &Writer, id: &str) -> Result<Thread, ApiError> {
        let n = w.read(id).map_err(|e| match e {
            writer::Error::NotFound(_) => not_a_thread(id),
            e => e.into(),
        })?;
        if n.type_name != THREAD_TYPE {
            return Err(not_a_thread(id));
        }
        let doc = doc_to_json(&n.doc);
        let f = &doc["fields"];
        let s = |k: &str| f[k].as_str().unwrap_or_default().to_owned();
        Ok(Thread {
            id: n.id,
            version: n.version,
            title: s("title"),
            agent: s("agent"),
            origin: s("origin"),
            last_activity: f["last_activity"].as_str().map(str::to_owned),
            archived: writer::is_archived(&n.doc),
            transcript: Transcript::parse(f["transcript"].as_str().unwrap_or_default()),
            doc,
        })
    }

    /// Thread `id` of agent `agent`: a thread of another agent is a 404.
    pub fn read_of(w: &Writer, agent: &str, id: &str) -> Result<Thread, ApiError> {
        let t = Thread::read(w, id)?;
        if t.agent != agent {
            return Err(not_a_thread(id));
        }
        Ok(t)
    }

    /// The thread as the list answers it.
    pub fn summary(&self) -> Value {
        let mut v = json!({
            "id": self.id,
            "title": self.title,
            "agent": self.agent,
            "origin": self.origin,
            "last_activity": self.last_activity,
            "pending": self.transcript.pending(),
            "archived": self.archived,
            "version": self.version,
        });
        put_running(&mut v, &self.id);
        v
    }

    /// The thread with its transcript, as `GET /api/agents/{name}/threads/{id}` answers it.
    pub fn json(&self) -> Value {
        let mut v = self.summary();
        v["messages"] = json!(self.transcript.messages);
        v["steps"] = Value::Object(self.transcript.steps.clone());
        v["proposals"] = json!(self.transcript.proposals);
        v
    }

    /// The note to write: its fields set from the thread, `last_activity` now, and `body`
    /// rendered again with `label` naming the agent.
    fn to_doc(&self, label: &str) -> Value {
        let mut doc = self.doc.clone();
        if !doc["fields"].is_object() {
            doc["fields"] = json!({});
        }
        let f = &mut doc["fields"];
        f["title"] = json!(self.title);
        f["agent"] = json!(self.agent);
        f["origin"] = json!(self.origin);
        f["last_activity"] = json!(now());
        f["pending"] = json!(self.transcript.pending());
        f["transcript"] =
            json!(serde_json::to_string(&self.transcript).unwrap_or_else(|_| "{}".to_owned()));
        let body = render_body(&self.transcript, label);
        let mut blocks: Vec<Value> = doc["blocks"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| !(b["kind"] == "content" && b["labels"][0] == "body"))
            .collect();
        if let Some(body) = body {
            blocks.push(body);
        }
        doc["blocks"] = Value::Array(blocks);
        doc
    }
}

/// Create a thread of agent `agent`, authored by the agent.
pub fn create(
    w: &Writer,
    agent: &str,
    title: Option<&str>,
    origin: &str,
) -> Result<Thread, ApiError> {
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(DEFAULT_TITLE);
    let doc = json!({
        "fields": {
            "title": title,
            "agent": agent,
            "origin": origin,
            "last_activity": now(),
            "pending": 0,
            "transcript": serde_json::to_string(&Transcript::default()).unwrap_or_default(),
        },
    });
    let saved = w.create(
        THREAD_TYPE,
        &Input::Json(doc),
        &Author::Agent(agent.to_owned()),
    )?;
    Thread::read(w, &saved.id)
}

/// Change thread `id` with `f` and write it as `author`, `label` naming the agent in its body.
/// The note is read again before each try, so a rename or an outcome recorded meanwhile is kept;
/// a write that meets a newer version tries again.
pub fn save(
    w: &Writer,
    id: &str,
    label: &str,
    author: Option<&Author>,
    f: impl Fn(&mut Thread),
) -> Result<Thread, ApiError> {
    let mut last = None;
    for _ in 0..RETRIES {
        let mut t = Thread::read(w, id)?;
        f(&mut t);
        let by = author
            .cloned()
            .unwrap_or_else(|| Author::Agent(t.agent.clone()));
        match w.update(id, &Input::Json(t.to_doc(label)), Some(&t.version), &by) {
            Ok(_) => return Thread::read(w, id),
            Err(e @ writer::Error::Conflict { .. }) => last = Some(e),
            Err(e) => return Err(e.into()),
        }
    }
    Err(last.map(ApiError::from).unwrap_or_else(|| {
        ApiError::new(StatusCode::CONFLICT, "conflict", "the thread kept changing")
    }))
}

/// The label of agent `name`, or the name when it has no file.
pub fn agent_label(w: &Writer, name: &str) -> String {
    w.agent(name)
        .map(|a| a.label)
        .unwrap_or_else(|_| name.to_owned())
}

/// Record the outcome of proposal `proposal_id` in thread `thread_id`.
pub fn record_outcome(
    w: &Writer,
    thread_id: &str,
    proposal_id: &str,
    status: Status,
    result: Option<Value>,
    error: Option<String>,
) -> Result<Thread, ApiError> {
    let t = Thread::read(w, thread_id)?;
    if !t.transcript.proposals.iter().any(|p| p.id() == proposal_id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("thread {thread_id} has no proposal {proposal_id}"),
        ));
    }
    let label = agent_label(w, &t.agent);
    save(w, thread_id, &label, None, |t| {
        if let Some(p) = t
            .transcript
            .proposals
            .iter_mut()
            .find(|p| p.id() == proposal_id)
        {
            p.status = status;
            p.result = result.clone();
            p.error = error.clone();
            p.reported = false;
        }
    })
}

/// A title from the user's first question: its first line, cut at [`TITLE_CHARS`].
pub fn title_of(question: &str) -> Option<String> {
    let line = question.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut t: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        t.push('…');
    }
    Some(t)
}

/// `s` with every inline markup character escaped, so it reads as plain text (D41).
pub(crate) fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '*' | '+' | '~' | '`' | '=' | '{' | '}' | '[' | ']'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn heading(text: &str) -> Value {
    json!({ "kind": "heading", "fields": { "level": 3, "text": plain(text) } })
}

fn paragraphs(text: &str, out: &mut Vec<Value>) {
    for p in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        out.push(json!({ "kind": "paragraph", "fields": { "text": plain(p) } }));
    }
}

fn bullets(items: &[String], out: &mut Vec<Value>) {
    if items.is_empty() {
        return;
    }
    let items: Vec<Value> = items
        .iter()
        .map(|t| json!({ "kind": "item", "fields": { "text": plain(t) } }))
        .collect();
    out.push(json!({ "kind": "list", "fields": { "style": "bullet" }, "blocks": items }));
}

/// A tool step as one line: its name and its input, cut short.
fn step_line(name: &str, input: &Value) -> String {
    let mut args = match input {
        Value::Object(o) if o.is_empty() => String::new(),
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "note" | "wcl"))
            .map(|(k, v)| match v {
                Value::String(s) => format!("{k}: {s}"),
                v => format!("{k}: {v}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
        v => v.to_string(),
    };
    if args.chars().count() > 120 {
        args = args.chars().take(120).collect::<String>() + "…";
    }
    if args.is_empty() {
        format!("Used {name}")
    } else {
        format!("Used {name} ({args})")
    }
}

/// A proposal as one line of the body, with where it stands.
fn proposal_line(p: &Stored) -> String {
    let what = proposal_what(&p.event, &p.data);
    match p.status {
        Status::Pending => format!("Proposed {what}: waiting for the user"),
        Status::Applied => format!("Proposed {what}: applied by the user"),
        Status::Discarded => format!("Proposed {what}: discarded by the user"),
        Status::Failed | Status::Stale => format!(
            "Proposed {what}: failed, {}",
            p.error.as_deref().unwrap_or("it did not apply")
        ),
        Status::Made => match p.data["op"].as_str() {
            Some(op) => format!("Made {op} {}", p.data["title"].as_str().unwrap_or_default())
                .trim_end()
                .to_owned(),
            None => "Made a change".to_owned(),
        },
    }
}

/// The readable conversation: the user's words and the agent's answers under headings, each
/// tool step and proposal one line. `None` for a thread with nothing said yet.
pub fn render_body(t: &Transcript, label: &str) -> Option<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut speaker: Option<bool> = None;
    for m in &t.messages {
        let tool_results = m
            .content
            .iter()
            .any(|c| matches!(c, Content::ToolResult { .. }));
        let user = m.role == Role::User && !tool_results;
        if speaker != Some(user) {
            out.push(heading(if user { "You" } else { label }));
            speaker = Some(user);
        }
        let mut lines: Vec<String> = Vec::new();
        for c in &m.content {
            match c {
                Content::Text { text } if user && is_outcome_lines(text) => {
                    lines.extend(text.lines().map(str::to_owned));
                }
                Content::Text { text } => {
                    bullets(&std::mem::take(&mut lines), &mut out);
                    paragraphs(text, &mut out);
                }
                Content::ToolCall { name, input, .. } => lines.push(step_line(name, input)),
                Content::ToolResult { id, .. } => {
                    lines.extend(
                        t.proposals
                            .iter()
                            .filter(|p| p.id() == id)
                            .map(proposal_line),
                    );
                }
                Content::Notice { text, .. } => lines.push(text.clone()),
            }
        }
        bullets(&lines, &mut out);
    }
    if out.is_empty() {
        return None;
    }
    Some(json!({ "kind": "content", "labels": ["body"], "blocks": out }))
}

/// A turn running on a thread (agents G17): its agent, its id in the turns form (`None` for a
/// trigger's run or a `POST /api/ai/chat` turn) and when it started, in milliseconds since the
/// epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub agent: String,
    pub turn: Option<String>,
    pub started_ms: i64,
}

/// The threads a turn is running on, by thread id: a second turn on one of them is refused.
fn busy() -> &'static Mutex<HashMap<String, Running>> {
    static BUSY: OnceLock<Mutex<HashMap<String, Running>>> = OnceLock::new();
    BUSY.get_or_init(Mutex::default)
}

fn held() -> std::sync::MutexGuard<'static, HashMap<String, Running>> {
    busy().lock().unwrap_or_else(PoisonError::into_inner)
}

/// The turn running on thread `id`, if one is.
pub fn running(id: &str) -> Option<Running> {
    held().get(id).cloned()
}

/// The turns running on agent `agent`'s threads.
pub fn running_of(agent: &str) -> Vec<Running> {
    held()
        .values()
        .filter(|r| r.agent == agent)
        .cloned()
        .collect()
}

/// Name the turn running on thread `id`: its id in the turns form.
pub fn set_turn(id: &str, turn: &str) {
    if let Some(r) = held().get_mut(id) {
        r.turn = Some(turn.to_owned());
    }
}

/// Holds a thread for its turn; dropping it frees the thread.
#[derive(Debug)]
pub struct Busy(String);

impl Busy {
    /// Hold thread `id` for a turn of agent `agent`, or 409 `thread_busy` when a turn runs on it.
    pub fn take(id: &str, agent: &str) -> Result<Busy, ApiError> {
        let mut held = held();
        if held.contains_key(id) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "thread_busy",
                "a turn is running on this thread: wait for it or stop it",
            ));
        }
        held.insert(
            id.to_owned(),
            Running {
                agent: agent.to_owned(),
                turn: None,
                started_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
        Ok(Busy(id.to_owned()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        held().remove(&self.0);
    }
}

/// `running`, `turn` and `started_ms` of thread `id`, as the list and the thread answer them.
fn put_running(v: &mut Value, id: &str) {
    let r = running(id);
    v["running"] = json!(r.is_some());
    v["turn"] = json!(r.as_ref().and_then(|r| r.turn.clone()));
    v["started_ms"] = json!(r.map(|r| r.started_ms));
}

type Shared = State<Arc<Writer>>;

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    /// `true` lists archived threads as well; absent or `false` leaves them out (agents G15).
    #[serde(default)]
    archived: Option<bool>,
}

/// `GET /api/agents/{name}/threads`: the agent's threads, newest activity first, without the
/// archived ones unless `?archived=true`.
pub async fn list(
    State(w): Shared,
    Path(name): Path<String>,
    Query(lq): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let include = if lq.archived.unwrap_or(false) {
        " include archived"
    } else {
        ""
    };
    if !schema::is_name(&name) {
        return Err(ApiError::bad_request(format!(
            "{name:?} is not an agent name"
        )));
    }
    let threads = blocking(w, move |w| {
        let q = format!(
            "from {THREAD_TYPE}{include} where agent == \"{name}\" sort last_activity desc \
             select title, origin, last_activity, pending, archived"
        );
        let mut out = Vec::new();
        let mut cursor = None;
        loop {
            let page = script_core::Vault::query_page(w, &q, query::MAX_PAGE, cursor.take())
                .map_err(ApiError::bad_request)?;
            for r in page["rows"].as_array().into_iter().flatten() {
                let f = &r["fields"];
                let mut row = json!({
                    "id": r["id"],
                    "title": r["title"],
                    "origin": f["origin"],
                    "last_activity": f["last_activity"],
                    "pending": f["pending"].as_u64().unwrap_or(0),
                    "archived": f["archived"].as_bool().unwrap_or(false),
                });
                if let Some(id) = r["id"].as_str() {
                    put_running(&mut row, id);
                }
                out.push(row);
            }
            match page["next"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        Ok::<_, ApiError>(out)
    })
    .await?;
    Ok(Json(json!({ "threads": threads })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewThread {
    #[serde(default)]
    title: Option<String>,
}

/// `POST /api/agents/{name}/threads`.
pub async fn start(
    State(w): Shared,
    Path(name): Path<String>,
    body: Option<Json<NewThread>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let title = body.and_then(|Json(b)| b.title);
    let t = blocking(w, move |w| {
        let agent = w.agent(&name)?;
        create(w, &agent.name, title.as_deref(), "user")
    })
    .await?;
    Ok((StatusCode::CREATED, Json(t.json())))
}

/// `GET /api/agents/{name}/threads/{id}`.
pub async fn read(
    State(w): Shared,
    Path((name, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let t = blocking(w, move |w| Thread::read_of(w, &name, &id)).await?;
    Ok(Json(t.json()))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    title: String,
}

/// `PATCH /api/agents/{name}/threads/{id}`.
pub async fn rename(
    State(w): Shared,
    Caller(author): Caller,
    Path((name, id)): Path<(String, String)>,
    Json(b): Json<Rename>,
) -> Result<Json<Value>, ApiError> {
    let title = b.title.trim().to_owned();
    if title.is_empty() {
        return Err(ApiError::bad_request("a thread's title cannot be empty"));
    }
    let t = blocking(w, move |w| {
        Thread::read_of(w, &name, &id)?;
        let label = agent_label(w, &name);
        save(w, &id, &label, Some(&author), |t| t.title = title.clone())
    })
    .await?;
    Ok(Json(t.json()))
}

/// `DELETE /api/agents/{name}/threads/{id}`: the note goes to the trash.
pub async fn delete(
    State(w): Shared,
    Caller(author): Caller,
    Path((name, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    blocking(w, move |w| {
        let t = Thread::read_of(w, &name, &id)?;
        w.delete(&t.id, Some(&t.version), &author)?;
        Ok::<_, ApiError>(json!({ "id": t.id, "deleted": true }))
    })
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(event: &str, data: Value, status: Status) -> Stored {
        Stored {
            event: event.into(),
            data,
            status,
            result: None,
            error: None,
            reported: false,
        }
    }

    #[test]
    fn outcome_lines_are_taken_once() {
        let mut t = Transcript::default();
        t.proposals.push(stored(
            "proposal",
            json!({ "id": "a", "op": "update", "note_id": "N" }),
            Status::Pending,
        ));
        t.proposals.push(Stored {
            result: Some(json!({ "version": "v3" })),
            ..stored(
                "proposal",
                json!({ "id": "b", "op": "update", "note_id": "M" }),
                Status::Applied,
            )
        });
        t.proposals.push(stored(
            "proposal",
            json!({ "id": "c", "op": "download", "url": "https://x/y.jpg" }),
            Status::Discarded,
        ));
        assert_eq!(t.pending(), 1);
        assert_eq!(
            t.take_outcome_lines(),
            [
                "[Applied by the user: edit of note M now at version v3]",
                "[Discarded by the user: download of https://x/y.jpg]"
            ]
        );
        assert!(t.take_outcome_lines().is_empty());
    }

    #[test]
    fn agent_outcome_lines_name_the_agent() {
        let mut t = Transcript::default();
        for (id, data, status) in [
            (
                "a",
                json!({ "kind": "agent", "op": "change", "name": "clerk", "exists": true }),
                Status::Applied,
            ),
            (
                "b",
                json!({ "kind": "agent", "op": "create", "name": "helper", "exists": false }),
                Status::Discarded,
            ),
            (
                "c",
                json!({ "kind": "agent", "op": "delete", "name": "old" }),
                Status::Applied,
            ),
        ] {
            let mut data = data;
            data["id"] = json!(id);
            t.proposals.push(stored("agent_proposal", data, status));
        }
        assert_eq!(
            t.take_outcome_lines(),
            [
                "[Applied by the user: changed agent clerk]",
                "[Discarded by the user: new agent helper]",
                "[Applied by the user: deleted agent old]"
            ]
        );
    }

    #[test]
    fn the_body_escapes_markup_and_names_the_speakers() {
        let t = Transcript {
            messages: vec![
                Message::user("Fix [[+task|x]] **now**"),
                Message {
                    role: Role::Assistant,
                    content: vec![Content::text("Done.")],
                    provider_state: None,
                },
            ],
            ..Transcript::default()
        };
        let body = render_body(&t, "Assistant").unwrap();
        let blocks = body["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["fields"]["text"], "You");
        assert_eq!(
            blocks[1]["fields"]["text"],
            "Fix \\[\\[\\+task|x\\]\\] \\*\\*now\\*\\*"
        );
        assert_eq!(blocks[2]["fields"]["text"], "Assistant");
        assert_eq!(blocks[3]["fields"]["text"], "Done.");
    }

    #[test]
    fn a_title_is_the_first_line_cut_short() {
        assert_eq!(title_of("\n  Hello there\nmore").unwrap(), "Hello there");
        let long = "x".repeat(70);
        assert_eq!(title_of(&long).unwrap().chars().count(), 61);
        assert_eq!(title_of("  "), None);
    }
}
