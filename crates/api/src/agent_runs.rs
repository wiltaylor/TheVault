//! Agent trigger runs (agents G9 to G12): the runner the scheduler starts them through,
//! `GET /api/agents/{name}/runs` and `GET /api/agents/calls/{call}`.
//!
//! [`TriggerRunner`] opens a thread (`origin = "trigger:<name>"`) and runs one unattended turn on
//! it with [`ai_chat::run_unattended`]: the agent is the caller, and the turn is offered only the
//! read tools its permissions allow and the change tools whose confirmation switch is off, so
//! nothing waits for a user. A call from a note and notes sent to the agent run through it too,
//! on a thread of origin `mention` or `dispatch` that names its notes: those runs are also
//! offered the change tools whose switch is on, and their proposals wait in the thread for the
//! user's Apply (TV-77 D2). Such a run that ends with the thread needing the user (a proposal
//! waits, or the agent asked a question) leaves one notification linking to the thread (TV-77
//! Q9). A model call that fails with a transient error is retried after the waits
//! [`crate::ai_retry`] sets out. Only the server installs it (`crate::router`); the embedded app
//! servers never run triggers.
//!
//! `GET /api/agents/{name}/runs` (read) answers `{tools, triggers}`: `tools`, the tools a trigger
//! run of the agent is offered, and for each trigger in file order `{name, kind, runs}`, `runs`
//! its last [`RUNS_SHOWN`] runs, newest first, each `{id, thread, started, ended, outcome,
//! message, log}` with times in milliseconds since the epoch and `log` the run's trail lines and
//! capacity waits, one per line, which grows while the run waits.
//!
//! `GET /api/agents/calls/{call}` (read) answers what became of a call committed in a note
//! (agents G19): `{agent, note, thread, running}`, `thread` null for a call that was not run and
//! `running` whether a turn runs on its thread now; 404 while the server has not seen the call.

use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use schema::{Agent, TriggerWhen};
use serde_json::{Value, json};
use writer::Writer;

use crate::{ApiError, agent_threads, ai_chat, ai_retry, blocking};

/// Reserve a call for an authenticated editor, including a paired app editor.
pub async fn issue_call(
    State(w): State<Arc<Writer>>,
    crate::auth::Caller(author): crate::auth::Caller,
    axum::Extension(crate::auth::GrantedScope(scope)): axum::Extension<crate::auth::GrantedScope>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    blocking(w, move |w| {
        let paired = match &author {
            state::Author::Token(name) => w.state().device_for_token(name)?.is_some(),
            _ => false,
        };
        let permitted = call_issuer_allowed(&author, scope, paired);
        if !permitted {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "calls require an editor session or a paired device write token",
            ));
        }
        let note = body["note"].as_str().unwrap_or("");
        let agent = body["agent"].as_str().unwrap_or("");
        if note.parse::<ulid::Ulid>().is_err() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid",
                "a note id is required",
            ));
        }
        let agents = w.agents()?;
        let valid = agents.iter().any(|(name, a)| {
            name == agent
                && a.as_ref().is_ok_and(|a| {
                    a.enabled
                        && w.read(note).map_or(true, |n| {
                            !schema::AGENT_CALL_EXCLUDED_TYPES.contains(&n.type_name.as_str())
                                && a.mention_trigger(&n.type_name).is_some()
                        })
                })
        });
        if !valid {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid",
                "this agent cannot be called from this note",
            ));
        }
        let call = ulid::Ulid::new().to_string();
        let (kind, name) = author.parts();
        w.state()
            .issue_agent_call(&call, note, agent, &format!("{kind}:{name}"))?;
        Ok(Json(json!({ "call": call })))
    })
    .await
}

fn call_issuer_allowed(author: &state::Author, scope: state::Scope, paired: bool) -> bool {
    match author {
        state::Author::Session(_) => true,
        state::Author::Token(name) => {
            paired && scope == state::Scope::Write && !name.starts_with("agent-")
        }
        _ => false,
    }
}

/// How many runs of each trigger the route answers.
pub const RUNS_SHOWN: u32 = 20;

/// Runs agent turns for the scheduler's triggers, on the server's async runtime.
pub struct TriggerRunner {
    writer: Arc<Writer>,
    handle: tokio::runtime::Handle,
    /// The waits before a model call is retried.
    waits: ai_retry::Waits,
}

impl TriggerRunner {
    /// A runner over `writer`, on the async runtime this is called in, or on one of its own when
    /// called outside one, retrying a model call at capacity after `waits`.
    pub fn new(writer: Arc<Writer>, waits: ai_retry::Waits) -> TriggerRunner {
        let handle = tokio::runtime::Handle::try_current().unwrap_or_else(|_| own_runtime());
        TriggerRunner {
            writer,
            handle,
            waits,
        }
    }
}

/// A runtime kept for the life of the process, for a runner made outside any runtime.
fn own_runtime() -> tokio::runtime::Handle {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("agent-runs")
                .enable_all()
                .build()
                .expect("start the agent runs' async runtime")
        })
        .handle()
        .clone()
}

impl scheduler::AgentRunner for TriggerRunner {
    fn open_thread(
        &self,
        agent: &Agent,
        origin: &str,
        title: &str,
        notes: &[String],
    ) -> Result<String, String> {
        agent_threads::create_with(&self.writer, &agent.name, Some(title), origin, notes)
            .map(|t| t.id)
            .map_err(|e| {
                e.body["message"]
                    .as_str()
                    .unwrap_or("the thread could not be created")
                    .to_owned()
            })
    }

    fn run_turn(&self, agent: &Agent, thread: &str, prompt: &str) -> scheduler::TurnReport {
        let w = self.writer.clone();
        // Open clients show the thread running while the run goes (agents G17).
        crate::ai_turns::publish(&w, &agent.name, thread, None, true);
        // This blocks the scheduler's thread for this run alone; a capacity wait sleeps on the
        // runtime's timer, holding no runtime thread.
        let done = self.handle.block_on(ai_chat::run_unattended(
            w,
            agent,
            thread,
            prompt,
            self.waits.clone(),
        ));
        crate::ai_turns::publish(&self.writer, &agent.name, thread, None, false);
        let outcome = done
            .ended
            .unwrap_or_else(|| Err("the turn stopped before it ended".to_owned()));
        // A failed run has its own notification from the scheduler.
        if outcome.is_ok() {
            notify_needs_you(&self.writer, agent, thread);
        }
        scheduler::TurnReport {
            outcome,
            log: done.log,
        }
    }
}

/// "<Agent> needs you: <title>" when a run a person asked for (a call from a note, or notes sent
/// to the agent) ended with its thread waiting for the user (TV-77 Q9), linking to the thread.
fn notify_needs_you(w: &Writer, agent: &Agent, thread: &str) {
    let Ok(t) = agent_threads::Thread::read(w, thread) else {
        return;
    };
    if ![scheduler::MENTION_ORIGIN, scheduler::DISPATCH_ORIGIN].contains(&t.origin.as_str()) {
        return;
    }
    let pending = t.transcript.pending();
    if agent_threads::state_of(false, &t.question, pending) != "needs_you" {
        return;
    }
    let body = if !t.question.is_empty() {
        t.question.clone()
    } else if pending == 1 {
        "A change waits for your Apply.".to_owned()
    } else {
        format!("{pending} changes wait for your Apply.")
    };
    let clip = |s: &str, n: usize| -> String {
        if s.chars().count() <= n {
            return s.to_owned();
        }
        s.chars().take(n - 1).chain(['\u{2026}']).collect()
    };
    let n = state::NewNotification {
        title: clip(
            &format!("{} needs you: {}", agent.label, t.title),
            state::MAX_TITLE,
        ),
        body: Some(clip(body.trim(), state::MAX_BODY)),
        note_id: None,
        url: Some(format!("/agents/{}/chat?thread={thread}", agent.name)),
        source: format!("agent:{}", agent.name),
    };
    if let Err(e) = w.state().add_notification(&n) {
        eprintln!(
            "agent {:?}: recording the needs-you notification: {e}",
            agent.name
        );
    }
}

/// `GET /api/agents/{name}/runs`.
pub async fn list(
    State(w): State<Arc<Writer>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let out = blocking(w, move |w| {
        let agent = w.agent(&name)?;
        let state = w.state();
        let mut triggers = Vec::new();
        for t in &agent.triggers {
            let runs = state.agent_runs(&agent.name, &t.name, RUNS_SHOWN)?;
            let runs: Vec<Value> = runs
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "thread": r.thread,
                        "started": r.started_ms,
                        "ended": r.ended_ms,
                        "outcome": r.outcome.as_str(),
                        "message": r.outcome.message(),
                        "log": r.log,
                    })
                })
                .collect();
            let kind = match t.when {
                TriggerWhen::Schedule(_) => "schedule",
                TriggerWhen::NoteEvent(_) => "event",
                TriggerWhen::Mention(_) => "mention",
            };
            triggers.push(json!({ "name": t.name, "kind": kind, "runs": runs }));
        }
        let queue = state.trigger_queue_stats(&name)?;
        Ok::<_, ApiError>(json!({
            "queue": { "waiting": queue.waiting, "failed": queue.failed, "expired": queue.expired, "dropped": queue.dropped },
            "tools": ai_chat::trigger_tools(w, &agent),
            "triggers": triggers,
        }))
    })
    .await?;
    Ok(Json(out))
}

/// Preview and confirm a backfill. An opaque, single-use ten-minute preview binds the
/// exact newest-N selection to the signed-in session, agent and trigger.
pub async fn backfill(
    State(w): State<Arc<Writer>>,
    crate::auth::Caller(author): crate::auth::Caller,
    Path((name, trigger)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !backfill_allowed(&author) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "session_required",
            "backfill requires a signed-in owner session",
        ));
    }
    let state::Author::Session(principal) = author else {
        unreachable!();
    };
    blocking(w, move |w| {
        let bad =
            |message: String| ApiError::new(StatusCode::BAD_REQUEST, "invalid_backfill", message);
        let scheduler = crate::script_tools::scheduler_for(w)
            .ok_or_else(|| bad("server has no scheduler".to_owned()))?;
        if let Some(preview) = body["preview"].as_str() {
            let payload = w
                .state()
                .confirm_trigger_backfill(preview, &principal, &name, &trigger)?
                .ok_or_else(|| bad("preview expired or already used".to_owned()))?;
            let notes: Vec<(String, String)> =
                serde_json::from_str(&payload).map_err(|e| bad(e.to_string()))?;
            let queued = scheduler
                .trigger_backfill(&name, &trigger, &notes)
                .map_err(bad)?;
            Ok::<_, ApiError>(Json(json!({ "queued": queued })))
        } else {
            let limit = body["limit"]
                .as_u64()
                .filter(|n| (1..=1000).contains(n))
                .ok_or_else(|| bad("limit must be 1 to 1000".to_owned()))?
                as usize;
            let mut notes = scheduler
                .trigger_backfill_candidates(&name, &trigger)
                .map_err(bad)?;
            let count = notes.len();
            notes.truncate(limit);
            let payload = serde_json::to_string(&notes).map_err(|e| bad(e.to_string()))?;
            let preview = w
                .state()
                .preview_trigger_backfill(&principal, &name, &trigger, &payload)?;
            Ok(Json(
                json!({ "count": count, "selected": notes.len(), "preview": preview }),
            ))
        }
    })
    .await
}

fn backfill_allowed(author: &state::Author) -> bool {
    matches!(author, state::Author::Session(_))
}

/// `GET /api/agents/calls/{call}`.
pub async fn call(
    State(w): State<Arc<Writer>>,
    Path(call): Path<String>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let found = blocking(w, move |w| {
        Ok::<_, ApiError>(match w.state().agent_call(&call)? {
            Some(answered) if query.get("note").is_none_or(|note| *note == answered.note) => {
                Some(answered)
            }
            _ => match query.get("note") {
                Some(note) => w.state().refused_agent_call(&call, note)?,
                None => None,
            },
        })
    })
    .await?;
    let Some(c) = found else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "the server has not seen this call yet",
        ));
    };
    let running = c
        .thread
        .as_deref()
        .is_some_and(|t| agent_threads::running(t).is_some());
    Ok(Json(json!({
        "agent": c.agent,
        "note": c.note,
        "thread": c.thread,
        "running": running,
    })))
}

#[cfg(test)]
mod call_issuer_tests {
    use super::call_issuer_allowed;
    use state::{Author, Scope};

    #[test]
    fn backfill_refuses_every_token_kind_and_agent_run() {
        assert!(super::backfill_allowed(&Author::Session("owner".into())));
        for author in [
            Author::Token("admin".into()),
            Author::Token("device-phone".into()),
            Author::Token("agent-clerk".into()),
            Author::Agent("clerk".into()),
            Author::AgentMail("clerk".into()),
            Author::Handler("import".into()),
            Author::Task("import".into()),
        ] {
            assert!(!super::backfill_allowed(&author), "{author:?}");
        }
    }

    #[test]
    fn only_editor_principals_issue_call_ids() {
        assert!(call_issuer_allowed(
            &Author::Session("owner".into()),
            Scope::Admin,
            false
        ));
        assert!(call_issuer_allowed(
            &Author::Token("device-phone".into()),
            Scope::Write,
            true
        ));
        for (author, scope, paired) in [
            (Author::Agent("assistant".into()), Scope::Write, true),
            (Author::Token("agent-assistant".into()), Scope::Write, true),
            (Author::Token("device-phone".into()), Scope::Read, true),
            (Author::Token("api".into()), Scope::Write, false),
            (Author::Token("api".into()), Scope::Admin, false),
            (Author::Handler("import".into()), Scope::Write, false),
            (Author::Task("import".into()), Scope::Write, false),
        ] {
            assert!(!call_issuer_allowed(&author, scope, paired), "{author:?}");
        }
    }
}
