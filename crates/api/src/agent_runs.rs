//! Agent trigger runs (agents G9 to G12): the runner the scheduler starts them through, and
//! `GET /api/agents/{name}/runs`.
//!
//! [`TriggerRunner`] opens a thread (`origin = "trigger:<name>"`) and runs one unattended turn on
//! it with [`ai_chat::run_unattended`]: the agent is the caller, and the turn is offered only the
//! read tools its permissions allow and the change tools whose confirmation switch is off, so
//! nothing waits for a user. Only the server installs it (`crate::router`); the embedded app
//! servers never run triggers.
//!
//! `GET /api/agents/{name}/runs` (read) answers `{tools, triggers}`: `tools`, the tools a trigger
//! run of the agent is offered, and for each trigger in file order `{name, kind, runs}`, `runs`
//! its last [`RUNS_SHOWN`] runs, newest first, each `{id, thread, started, ended, outcome,
//! message}` with times in milliseconds since the epoch.

use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::extract::{Path, State};
use schema::{Agent, TriggerWhen};
use serde_json::{Value, json};
use writer::Writer;

use crate::{ApiError, agent_threads, ai_chat, blocking};

/// How many runs of each trigger the route answers.
pub const RUNS_SHOWN: u32 = 20;

/// Runs agent turns for the scheduler's triggers, on the server's async runtime.
pub struct TriggerRunner {
    writer: Arc<Writer>,
    handle: tokio::runtime::Handle,
}

impl TriggerRunner {
    /// A runner over `writer`, on the async runtime this is called in, or on one of its own when
    /// called outside one.
    pub fn new(writer: Arc<Writer>) -> TriggerRunner {
        let handle = tokio::runtime::Handle::try_current().unwrap_or_else(|_| own_runtime());
        TriggerRunner { writer, handle }
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
    fn open_thread(&self, agent: &Agent, origin: &str, title: &str) -> Result<String, String> {
        agent_threads::create(&self.writer, &agent.name, Some(title), origin)
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
        let done = self
            .handle
            .block_on(ai_chat::run_unattended(w, agent, thread, prompt));
        scheduler::TurnReport {
            outcome: done
                .ended
                .unwrap_or_else(|| Err("the turn stopped before it ended".to_owned())),
            log: done.log,
        }
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
                    })
                })
                .collect();
            let kind = match t.when {
                TriggerWhen::Schedule(_) => "schedule",
                TriggerWhen::NoteEvent(_) => "event",
            };
            triggers.push(json!({ "name": t.name, "kind": kind, "runs": runs }));
        }
        Ok::<_, ApiError>(json!({
            "tools": ai_chat::trigger_tools(w, &agent),
            "triggers": triggers,
        }))
    })
    .await?;
    Ok(Json(out))
}
