//! The turns form of the AI chat and the apply route (ai D23, agents G17).
//!
//! `POST /api/ai/chat` streams its answer in the response to the POST, which a client that
//! buffers request bodies (the Android webview's IPC) cannot read as it comes, and which ends the
//! turn when the client goes. The turns form splits it in two, and every client uses it:
//!
//! - `POST /api/ai/chat/turns` takes the body `POST /api/ai/chat` takes, starts the turn and
//!   answers 201 `{ turn, started_ms }`.
//! - `GET /api/ai/chat/turns/{turn}/events?after=<n>` streams that turn's events, the same events
//!   as `POST /api/ai/chat` sends (see [`crate::ai_chat`]), from the one after the `n`th on: those
//!   sent so far, then the rest as they come, ending with the turn. Each event's SSE `id` is its
//!   number, counting from 1, and a reconnecting `EventSource`'s `Last-Event-ID` is read as
//!   `after`. Any number of readers may read one turn, each from where it chooses.
//! - `DELETE /api/ai/chat/turns/{turn}` stops it, 204, and forgets it.
//!
//! The server reads each turn's events itself, so a turn never waits for a reader (agents G17):
//! a turn on a thread (`thread` in the body) runs until it ends or is stopped, writing to its
//! thread note as it goes, whoever reads it. A turn with no thread, which nothing keeps, stops
//! once nobody has read it for [`READER_WAIT`], and waits for its first reader as
//! `POST /api/ai/chat` waits for its client. A finished turn's events are kept for
//! [`KEEP_FINISHED`]; after that they answer 410 `turn_expired`, and the client reads the thread
//! note, which holds the whole turn. Turns are held in memory, by id, for the caller that started
//! them: another caller's turn is a 404. At most [`MAX_TURNS`] run per caller at once, and per
//! agent, counting its trigger runs; one more is a 429 `too_many_turns`. A turn on a thread
//! starting and ending is published on `/api/events` as `ai_turn {agent, thread, turn, running}`.
//! A sub agent's turn (subagents U6) is read and stopped through the same routes, by the caller
//! of the turn that started it: its id is its session's running `turn`, and stopping it stops
//! only it, its parent answered that the user stopped it.
//! Both forms share one turn runner, [`ai_chat::start`].
//!
//! `POST /api/ai/apply` applies a change the chat proposed, its body the `proposal` or
//! `type_proposal` event's data as the client received it (a type proposal is told apart by its
//! WCL `after` text), a `taxonomy_proposal`'s (its `kind` is `taxonomy`, ai D31), or an
//! `agent_proposal`'s (its `kind` is `agent`, agents G20, checked again against its `proposer`), or an
//! `agent_memory_proposal`'s (its `kind` is `agent_memory`, agents G21, checked again against
//! its `proposer`, needing only `write` since memories are notes), or a `skill_proposal`'s (its
//! `kind` is `skill`, skills S9, answering 409 `stale` when the skill changed since), through
//! [`ai_chat::apply_change`], the code the confirmation switches use (ai D22), as the caller,
//! version-checked against its `base_version`. It needs `write`, and `admin` for a type,
//! taxonomy, agent or skill change: without it, 403 `needs_admin`. It answers `{op, note_id, version}`
//! for a note, `{op, type_name}` for a type, `{op, taxonomy_name, notes}` for a taxonomy and
//! `{op, agent_name}` for an agent and `{op, memory_id, memory_agent, version}` for a memory; an
//! agent or memory change the proposer may not make is a 403 `not_permitted`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{Author, LiveEvent, Scope};
use tokio::sync::{Notify, watch};

use crate::ApiError;
use crate::agent_threads::{self, Status};
use crate::ai_chat::{self, ChatBody, Sent};
use crate::ai_settings::Ai;
use crate::auth::{Caller, GrantedScope};

/// How long a turn with no thread runs with nobody reading it before it stops.
pub const READER_WAIT: Duration = Duration::from_secs(30);

/// How long a finished turn's events are kept for a reader that comes back (agents G17).
pub const KEEP_FINISHED: Duration = Duration::from_secs(10 * 60);

/// The most turns one caller, or one agent, runs at once.
pub const MAX_TURNS: usize = 4;

/// The message of a type change applied without `admin`.
pub const NEEDS_ADMIN_MESSAGE: &str = "Apply type changes in the web app.";

/// The 403 a taxonomy change without `admin` answers (ai D31).
pub const NEEDS_ADMIN_TAXONOMY_MESSAGE: &str = "Apply taxonomy changes in the web app.";

/// The 403 an agent change without `admin` answers (agents G20).
pub const NEEDS_ADMIN_AGENT_MESSAGE: &str = "Apply agent changes in the web app.";

/// The 403 a skill change without `admin` answers (skills S9).
pub const NEEDS_ADMIN_SKILL_MESSAGE: &str = "Apply skill changes in the web app.";

/// Why a caller without `admin` cannot apply a script tool proposal (script-tools T3).
pub const NEEDS_ADMIN_TOOL_MESSAGE: &str = "Apply script tool changes in the web app.";

/// Why a caller without `admin` cannot apply an agent change that turns on Run wscript code
/// (script-tools T1).
pub const NEEDS_ADMIN_RUN_CODE_MESSAGE: &str =
    "Only an admin turns on Run wscript code: apply this agent change in the web app.";

/// Why a caller without `admin` cannot apply an agent change that turns on sub agents or adds an
/// agent to delegate to (subagents U2).
pub const NEEDS_ADMIN_SUBAGENTS_MESSAGE: &str =
    "Only an admin grants sub agents: apply this agent change in the web app.";

/// The most expired turn ids remembered, to answer 410 rather than 404.
const EXPIRED_KEPT: usize = 10_000;

/// How long finished turns are kept: [`KEEP_FINISHED`], or in a test build the milliseconds
/// `VAULT_TEST_AI_TURN_KEEP_MS` names.
fn keep_finished() -> Duration {
    if cfg!(debug_assertions)
        && let Some(ms) = std::env::var("VAULT_TEST_AI_TURN_KEEP_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    KEEP_FINISHED
}

/// The turns started, running or finished and kept, by id.
#[derive(Default)]
pub struct Turns {
    live: Mutex<HashMap<String, Arc<Held>>>,
    /// The turns whose events are no longer kept.
    expired: Mutex<HashSet<String>>,
}

/// One turn and the events it has sent.
struct Held {
    owner: Author,
    /// The agent running it: the built-in Assistant when the body named none.
    agent: String,
    thread: Option<String>,
    /// Set true to stop the turn.
    stop: watch::Sender<bool>,
    /// False once the turn is over.
    running: Arc<AtomicBool>,
    log: Mutex<Log>,
    /// Bumped whenever an event comes or the turn ends.
    changed: watch::Sender<u64>,
    /// Told when a reader comes.
    attached: Notify,
}

#[derive(Default)]
struct Log {
    events: Vec<Sent>,
    done: bool,
    readers: usize,
    /// Since when nobody has read the turn.
    unread_since: Option<Instant>,
}

impl Held {
    fn log(&self) -> MutexGuard<'_, Log> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n += 1);
    }
}

impl Turns {
    fn live(&self) -> MutexGuard<'_, HashMap<String, Arc<Held>>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn expired(&self) -> MutexGuard<'_, HashSet<String>> {
        self.expired.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Turn `id`, when `author` started it and it is kept: 410 once its events are let go.
    fn get(&self, id: &str, author: &Author) -> Result<Arc<Held>, ApiError> {
        if let Some(h) = self.live().get(id).filter(|h| h.owner == *author) {
            return Ok(h.clone());
        }
        if self.expired().contains(id) {
            return Err(ApiError::new(
                StatusCode::GONE,
                "turn_expired",
                "this chat turn ended a while ago: read its thread instead",
            ));
        }
        Err(unknown_turn())
    }

    /// Forget turn `id`, stopping it if it still runs.
    fn drop_turn(&self, id: &str) {
        if let Some(held) = self.live().remove(id) {
            let _ = held.stop.send(true);
        }
    }

    /// Let finished turn `id`'s events go, so a reader gets 410.
    fn expire(&self, id: &str) {
        if self.live().remove(id).is_none() {
            return;
        }
        let mut expired = self.expired();
        if expired.len() >= EXPIRED_KEPT {
            expired.clear();
        }
        expired.insert(id.to_owned());
    }
}

/// The sub agent turns (subagents U6): read and stopped through the turn routes like the turns
/// `POST /api/ai/chat/turns` starts, by the caller of the turn that started them.
fn subs() -> &'static Turns {
    static SUBS: std::sync::OnceLock<Turns> = std::sync::OnceLock::new();
    SUBS.get_or_init(Turns::default)
}

/// Turn `id` of `author`'s: one they started, or a sub agent turn one of theirs started.
fn find(a: &Ai, id: &str, author: &Author) -> Result<Arc<Held>, ApiError> {
    match a.turns.get(id, author) {
        Err(e) if e.status == StatusCode::NOT_FOUND => subs().get(id, author),
        got => got,
    }
}

/// A sub agent's turn (subagents U6): its events kept as a turn's are, for the session pane to
/// read through `GET /api/ai/chat/turns/{turn}/events`, and stopped through its `DELETE`. A sub
/// agent of an unattended run, which nobody follows, is kept here unlisted.
pub(crate) struct SubTurn {
    id: String,
    held: Arc<Held>,
    listed: bool,
}

impl SubTurn {
    /// Hold the turn of agent `agent` on sub agent session `thread`, for `owner`, which `stop`
    /// stops. Listed, its id is the session's running turn.
    pub(crate) fn start(
        owner: Author,
        agent: &str,
        thread: &str,
        stop: watch::Sender<bool>,
        running: Arc<AtomicBool>,
        listed: bool,
    ) -> SubTurn {
        let id = state::random_hex(16);
        let held = Arc::new(Held {
            owner,
            agent: agent.to_owned(),
            thread: Some(thread.to_owned()),
            stop,
            running,
            log: Mutex::new(Log::default()),
            changed: watch::Sender::new(0),
            attached: Notify::new(),
        });
        if listed {
            subs().live().insert(id.clone(), held.clone());
            agent_threads::set_turn(thread, &id);
        }
        SubTurn { id, held, listed }
    }

    /// Its id in the turn routes, when it is listed there.
    pub(crate) fn id(&self) -> Option<&str> {
        self.listed.then_some(self.id.as_str())
    }

    /// Keep event `e` for the turn's readers.
    pub(crate) fn push(&self, e: Sent) {
        self.held.log().events.push(e);
        self.held.bump();
    }

    /// Stop the turn.
    pub(crate) fn stop(&self) {
        let _ = self.held.stop.send(true);
    }

    /// The turn is over: its readers' streams end, and its events are let go after
    /// [`KEEP_FINISHED`].
    pub(crate) fn finish(self) {
        self.held.log().done = true;
        self.held.bump();
        if self.listed {
            let id = self.id;
            tokio::spawn(async move {
                tokio::time::sleep(keep_finished()).await;
                subs().expire(&id);
            });
        }
    }
}

fn unknown_turn() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_turn",
        "there is no such chat turn: it was stopped, or nobody read it",
    )
}

fn too_many(whose: &str) -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_turns",
        format!("{MAX_TURNS} chat turns {whose} are running already; stop one or wait for it"),
    )
}

/// Tell open clients a turn on `thread` started or ended (agents G17).
pub(crate) fn publish(
    w: &writer::Writer,
    agent: &str,
    thread: &str,
    turn: Option<&str>,
    running: bool,
) {
    w.state().events().publish(LiveEvent::AiTurn {
        agent: agent.to_owned(),
        thread: thread.to_owned(),
        turn: turn.map(str::to_owned),
        running,
    });
}

/// `POST /api/ai/chat/turns`.
pub async fn start(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Json(body): Json<ChatBody>,
) -> Result<impl IntoResponse, ApiError> {
    let id = state::random_hex(16);
    let agent = body.agent().unwrap_or(schema::ASSISTANT_AGENT).to_owned();
    let thread = body.thread().map(str::to_owned);
    {
        let live = a.turns.live();
        let running: Vec<&Arc<Held>> = live
            .values()
            .filter(|h| h.running.load(Ordering::SeqCst))
            .collect();
        if running.iter().filter(|h| h.owner == author).count() >= MAX_TURNS {
            return Err(too_many("of yours"));
        }
        // The turns on the agent's threads, in any form: its trigger runs too.
        if thread.is_some() && agent_threads::running_of(&agent).len() >= MAX_TURNS {
            return Err(too_many(&format!("of {agent}")));
        }
    }
    let (stop, stop_rx) = watch::channel(false);
    let (mut rx, running) =
        ai_chat::start(a.writer.clone(), author.clone(), scope, body, stop_rx).await?;
    let started_ms = thread
        .as_deref()
        .and_then(|t| {
            agent_threads::set_turn(t, &id);
            agent_threads::running(t)
        })
        .map_or_else(|| chrono::Utc::now().timestamp_millis(), |r| r.started_ms);
    let held = Arc::new(Held {
        owner: author,
        agent: agent.clone(),
        thread: thread.clone(),
        stop,
        running,
        log: Mutex::new(Log {
            unread_since: Some(Instant::now()),
            ..Log::default()
        }),
        changed: watch::Sender::new(0),
        attached: Notify::new(),
    });
    a.turns.live().insert(id.clone(), held.clone());
    if let Some(t) = &thread {
        publish(&a.writer, &agent, t, Some(&id), true);
    }
    // The server reads a thread turn's events itself, so the turn never waits for a reader. A
    // turn with no thread waits for its first.
    let turns = a.clone();
    let turn = id.clone();
    let pumped = held.clone();
    tokio::spawn(async move {
        if pumped.thread.is_none() {
            let mut stopped = pumped.stop.subscribe();
            tokio::select! {
                _ = pumped.attached.notified() => {}
                _ = stopped.wait_for(|s| *s) => {}
            }
        }
        while let Some(e) = rx.recv().await {
            pumped.log().events.push(e);
            pumped.bump();
        }
        pumped.log().done = true;
        pumped.bump();
        if let Some(t) = &pumped.thread {
            publish(&turns.writer, &pumped.agent, t, Some(&turn), false);
        }
        drop(pumped);
        tokio::time::sleep(keep_finished()).await;
        turns.turns.expire(&turn);
    });
    // A turn with no thread, which nothing keeps, stops once nobody reads it.
    if thread.is_none() {
        let turns = a.clone();
        let turn = id.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let unread = {
                    let log = held.log();
                    if log.done {
                        return;
                    }
                    log.readers == 0 && log.unread_since.is_some_and(|t| t.elapsed() >= READER_WAIT)
                };
                if unread {
                    turns.turns.drop_turn(&turn);
                    return;
                }
            }
        });
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({ "turn": id, "started_ms": started_ms })),
    ))
}

/// One reader of a turn, counted while it reads.
struct Reading(Arc<Held>);

impl Reading {
    fn new(held: Arc<Held>) -> Reading {
        held.log().readers += 1;
        held.attached.notify_one();
        Reading(held)
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        let mut log = self.0.log();
        log.readers -= 1;
        if log.readers == 0 {
            log.unread_since = Some(Instant::now());
        }
    }
}

/// The query of `GET /api/ai/chat/turns/{turn}/events`.
#[derive(Debug, Default, Deserialize)]
pub struct After {
    /// How many of the turn's events the reader has: the stream starts with the next.
    #[serde(default)]
    after: Option<usize>,
}

/// `GET /api/ai/chat/turns/{turn}/events`.
pub async fn events(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Path(id): Path<String>,
    Query(q): Query<After>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let held = find(&a, &id, &author)?;
    let after = q.after.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok())
    });
    let changed = held.changed.subscribe();
    let from = (Reading::new(held), changed, after.unwrap_or(0));
    let events = futures_util::stream::unfold(from, |(reading, mut changed, at)| async move {
        loop {
            changed.borrow_and_update();
            {
                let log = reading.0.log();
                if let Some(e) = log.events.get(at) {
                    let event = e.event().id((at + 1).to_string());
                    drop(log);
                    let next = (reading, changed, at + 1);
                    return Some((Ok::<_, std::convert::Infallible>(event), next));
                }
                if log.done {
                    return None;
                }
            }
            if changed.changed().await.is_err() {
                return None;
            }
        }
    });
    Ok(ai_chat::sse(events))
}

/// `DELETE /api/ai/chat/turns/{turn}`.
pub async fn stop(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    find(&a, &id, &author)?;
    // A sub agent's turn stops alone: its parent gets "stopped by the user" (subagents U6).
    a.turns.drop_turn(&id);
    subs().drop_turn(&id);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/ai/apply`.
pub async fn apply(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Json(mut change): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !change.is_object() {
        return Err(ApiError::bad_request(
            "send the proposal as the chat sent it",
        ));
    }
    // A taxonomy or agent proposal says so in `kind` (ai D31, agents G20), as does a type delete; a
    // type proposal's `after` is the type file's WCL text; a note proposal's is a note.
    let event = if change["kind"] == "taxonomy" {
        "taxonomy_proposal"
    } else if change["kind"] == "agent" {
        "agent_proposal"
    } else if change["kind"] == "skill" {
        "skill_proposal"
    } else if change["kind"] == "tool" {
        "tool_proposal"
    } else if change["kind"] == "agent_memory" {
        "agent_memory_proposal"
    } else if change["kind"] == "type"
        || (change["after"].is_string() && change.get("op").is_none())
    {
        "type_proposal"
    } else {
        "proposal"
    };
    if !scope.allows(Scope::Admin) {
        let message = match event {
            "type_proposal" => Some(NEEDS_ADMIN_MESSAGE),
            "taxonomy_proposal" => Some(NEEDS_ADMIN_TAXONOMY_MESSAGE),
            "agent_proposal" => Some(NEEDS_ADMIN_AGENT_MESSAGE),
            "skill_proposal" => Some(NEEDS_ADMIN_SKILL_MESSAGE),
            "tool_proposal" => Some(NEEDS_ADMIN_TOOL_MESSAGE),
            _ => None,
        };
        if let Some(message) = message {
            // An agent's proposal waiting in its thread is the user's to apply from an app, whose
            // token has `write`: the change applied is the one the thread holds, not the body, so
            // a caller without `admin` applies only what an agent proposed.
            match waiting_in_thread(&a.writer, event, &change) {
                Some(stored) => change = stored,
                None => return Err(ApiError::new(StatusCode::FORBIDDEN, "needs_admin", message)),
            }
        }
    }
    // Turning on an agent's Run wscript code is full access to the vault: only the signed-in
    // admin grants it, never a write token applying what an agent proposed (script-tools T1).
    // Granting sub agents is guarded the same way (subagents U2).
    if event == "agent_proposal" && !scope.allows(Scope::Admin) {
        let w = a.writer.clone();
        let c = change.clone();
        let grants = tokio::task::spawn_blocking(move || {
            if crate::ai_agents::grants_run_code(&w, &c) {
                Some(NEEDS_ADMIN_RUN_CODE_MESSAGE)
            } else if crate::ai_agents::grants_subagents(&w, &c) {
                Some(NEEDS_ADMIN_SUBAGENTS_MESSAGE)
            } else {
                None
            }
        })
        .await
        .unwrap_or(Some(NEEDS_ADMIN_RUN_CODE_MESSAGE));
        if let Some(message) = grants {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "needs_admin", message));
        }
    }
    let w = a.writer.clone();
    tokio::task::spawn_blocking(move || apply_as_agent(&w, author, event, &change))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map(Json)
}

/// Apply `change` as the caller, or, when it names the agent that proposed it, as that agent
/// (agents G11) within its note types (agents G4); with its `thread`, record the outcome there
/// (agents G8).
fn apply_as_agent(
    w: &writer::Writer,
    caller: Author,
    event: &str,
    change: &Value,
) -> Result<Value, ApiError> {
    let author = match change["agent"].as_str().filter(|n| !n.is_empty()) {
        Some(name) => {
            let agent = w.agent(name)?;
            if event == "proposal" {
                let limits = ai_chat::Limits::of(&agent);
                let type_name = match change["op"].as_str().unwrap_or("update") {
                    "download" => None,
                    "create" => change["type"].as_str().map(str::to_owned),
                    _ => change["note_id"]
                        .as_str()
                        .and_then(|id| w.read(id).ok())
                        .map(|n| n.type_name),
                };
                if let Some(t) = type_name
                    && !limits.allows(&t)
                {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "not_permitted",
                        format!("{} may not change {t} notes.", agent.label),
                    ));
                }
            }
            // A throwaway sub agent's proposal is applied as it, `<agent>/sub:<n>` (subagents
            // U4).
            match change["sub"].as_u64() {
                Some(n) => Author::Agent(format!("{}/sub:{n}", agent.name)),
                None => Author::Agent(agent.name),
            }
        }
        None => caller,
    };
    let result = ai_chat::apply_change(w, &author, event, change);
    crate::ai_activity::applied(
        w,
        &author,
        event,
        change,
        result.as_ref().err().map(ToString::to_string).as_deref(),
    );
    if let (Some(thread), Some(id)) = (change["thread"].as_str(), change["id"].as_str()) {
        let recorded = match &result {
            Ok(done) => agent_threads::record_outcome(
                w,
                thread,
                id,
                Status::Applied,
                Some(done.clone()),
                None,
            ),
            Err(e) => {
                let stale = matches!(
                    e,
                    ai_chat::ApplyError::Stale(_)
                        | ai_chat::ApplyError::Write(
                            writer::Error::Conflict { .. } | writer::Error::NotFound(_)
                        )
                );
                let status = if stale { Status::Stale } else { Status::Failed };
                agent_threads::record_outcome(w, thread, id, status, None, Some(e.to_string()))
            }
        };
        if let Err(e) = recorded {
            eprintln!(
                "the outcome of proposal {id} was not recorded in thread {thread}: {}",
                e.body
            );
        }
    }
    result.map_err(ApiError::from)
}

/// The body of `POST /api/ai/discard`: the proposal as the chat sent it, which names its thread.
#[derive(Debug, serde::Deserialize)]
pub struct Discard {
    id: String,
    thread: String,
}

/// The proposal `change` names, as its thread holds it, when it is an `event` proposal still
/// waiting for the user there.
fn waiting_in_thread(w: &writer::Writer, event: &str, change: &Value) -> Option<Value> {
    let thread = change["thread"].as_str().filter(|s| !s.is_empty())?;
    let id = change["id"].as_str().filter(|s| !s.is_empty())?;
    let t = agent_threads::Thread::read(w, thread).ok()?;
    t.transcript
        .proposals
        .iter()
        .find(|p| p.id() == id && p.event == event && p.waits())
        .map(|p| p.data.clone())
}

/// `POST /api/ai/discard` (read): the user discarded a proposal of a thread, recorded there so
/// the reopened thread shows it and the next turn tells the model (agents G8, ai D29). It
/// changes nothing else.
pub async fn discard(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Json(b): Json<Discard>,
) -> Result<Json<Value>, ApiError> {
    let w = a.writer.clone();
    let t = crate::blocking(w, move |w| {
        let t = agent_threads::record_outcome(w, &b.thread, &b.id, Status::Discarded, None, None)?;
        crate::ai_activity::discarded(w, &author, &t.id, &b.id, &t.agent);
        Ok::<_, ApiError>(t)
    })
    .await?;
    Ok(Json(
        json!({ "thread": t.id, "pending": t.transcript.pending() }),
    ))
}
