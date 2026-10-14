//! Scheduled tasks, their run history, and the secrets they read (D10).
//!
//! - `PUT /api/tasks/{name}` takes the task file as WCL text, or JSON `{ "wcl": "..." }`. It is
//!   checked against `docs/vault-format.md` "Task files", and the script it names must exist.
//! - `POST /api/tasks/{name}/run` runs the task now, paused or not, and answers with the run once
//!   it ends. A failure schedules retries as a scheduled run's does.
//! - `GET /api/cron?schedule=..&timezone=..&count=..` describes a cron schedule in words and lists
//!   the next times it fires, so the editor previews exactly what the scheduler will do.
//! - `PUT /api/secrets/{name}` takes `{ "value": "..." }`, or the value as a text body. No response
//!   ever carries a secret's value: scripts read secrets, clients only write them.
//! - `/api/test/clock` exists only when the server runs with `--test-clock` (S7). `GET` reads the
//!   clock; `POST { "advance": "<ISO 8601 duration>" }` or `{ "set": "<RFC 3339 time>" }` moves it,
//!   runs whatever became due, and answers with those runs once they end.
//!
//! Times are milliseconds since the Unix epoch, as everywhere else in the API.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use scheduler::{Scheduler, TaskStatus, TestClock};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{RunOutcome, SecretInfo, TaskRun};
use writer::Writer;

use crate::defs::{body, text_or_json};
use crate::{ApiError, blocking};

type Tasks = State<Arc<Scheduler>>;

/// The most runs one history request returns.
const MAX_RUNS: u32 = 500;

impl From<scheduler::Error> for ApiError {
    fn from(e: scheduler::Error) -> ApiError {
        use scheduler::Error as E;
        let message = e.to_string();
        match e {
            E::NoTask(_) => ApiError::new(StatusCode::NOT_FOUND, "not_found", message),
            E::Broken { .. } => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "task_invalid", message)
            }
            E::Running(_) => ApiError::new(StatusCode::CONFLICT, "task_running", message),
            E::Writer(e) => e.into(),
            E::State(e) => e.into(),
        }
    }
}

/// Run blocking work against the scheduler off the async runtime.
async fn on_scheduler<T: Send + 'static>(
    s: Arc<Scheduler>,
    f: impl FnOnce(&Scheduler) -> scheduler::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || f(&s))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(Into::into)
}

fn check_name(what: &str, name: &str) -> Result<(), ApiError> {
    if schema::is_name(name) {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "{name:?} is not a valid {what} name: use [a-z][a-z0-9_]*, at most 64 characters"
        )))
    }
}

pub(crate) fn run_json(r: &TaskRun) -> Value {
    let (outcome, message) = match &r.outcome {
        RunOutcome::Running => ("running", None),
        RunOutcome::Succeeded => ("succeeded", None),
        RunOutcome::Failed(m) => ("failed", Some(m.as_str())),
    };
    json!({
        "id": r.id,
        "task": r.task,
        "attempt": r.attempt,
        "started": r.started_ms,
        "ended": r.ended_ms,
        "outcome": outcome,
        "message": message,
        "log": r.log,
    })
}

fn task_json(s: &TaskStatus) -> Value {
    let mut out = json!({
        "name": s.name,
        "next_run": s.next_run_ms,
        "retry": s.retry.map(|(attempt, at)| json!({ "attempt": attempt, "at": at })),
        "running": s.running,
        "failing": s.failing(),
        "latest_run": s.latest_run.as_ref().map(run_json),
    });
    match &s.def {
        Ok(d) => {
            let def = json!({
                "label": d.label,
                "description": d.description,
                "schedule": d.schedule,
                "schedule_text": d.schedule_text(),
                "timezone": d.timezone.name(),
                "script": d.script,
                "function": d.function,
                "allow_domains": d.allow_domains,
                "secrets": d.secrets,
                "read_paths": d.read_paths,
                "write_paths": d.write_paths,
                "retries": d.retries,
                "retry_delay_ms": d.retry_delay.as_millis() as u64,
                "timeout_ms": d.timeout.as_millis() as u64,
                "paused": d.paused,
            });
            if let (Some(out), Some(def)) = (out.as_object_mut(), def.as_object()) {
                out.extend(def.clone());
            }
        }
        Err(message) => out["error"] = json!(message),
    }
    out
}

// ---- tasks ----------------------------------------------------------------------------------

/// `GET /api/tasks`: every task with when it runs next and how its latest run went.
pub async fn list(State(s): Tasks) -> Result<Json<Value>, ApiError> {
    let tasks = on_scheduler(s, |s| s.statuses()).await?;
    let tasks: Vec<Value> = tasks.iter().map(task_json).collect();
    Ok(Json(json!({ "tasks": tasks })))
}

/// `GET /api/tasks/{name}`.
pub async fn read(
    State(s): Tasks,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    check_name("task", &name)?;
    let (status, wcl) = on_scheduler(s, move |s| {
        let wcl = s.writer().read_task(&name)?;
        Ok((s.status(&name)?, wcl))
    })
    .await?;
    let mut json = task_json(&status);
    json["wcl"] = json!(wcl);
    Ok(text_or_json(&headers, wcl, json))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskBody {
    wcl: String,
}

/// `PUT /api/tasks/{name}`: create or replace a task file.
pub async fn put(
    State(s): Tasks,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: TaskBody = body(&headers, &raw, |wcl| TaskBody { wcl })?;
    let (status, wcl) = on_scheduler(s, move |s| {
        let wcl = s.put_task(&name, &b.wcl)?;
        Ok((s.status(&name)?, wcl))
    })
    .await?;
    let mut json = task_json(&status);
    json["wcl"] = json!(wcl);
    Ok(text_or_json(&headers, wcl, json))
}

/// `DELETE /api/tasks/{name}`. The task's run history stays.
pub async fn delete(State(s): Tasks, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    on_scheduler(s, move |s| s.delete_task(&n)).await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

#[derive(Deserialize)]
pub struct RunsQuery {
    limit: Option<u32>,
}

/// `GET /api/tasks/{name}/runs`: the task's runs, newest first, at most `?limit=` of them.
pub async fn runs(
    State(s): Tasks,
    Path(name): Path<String>,
    Query(q): Query<RunsQuery>,
) -> Result<Json<Value>, ApiError> {
    check_name("task", &name)?;
    let limit = q.limit.unwrap_or(50).clamp(1, MAX_RUNS);
    let n = name.clone();
    let runs = on_scheduler(s, move |s| Ok(s.writer().state().runs(&n, limit)?)).await?;
    let runs: Vec<Value> = runs.iter().map(run_json).collect();
    Ok(Json(json!({ "task": name, "runs": runs })))
}

/// `GET /api/tasks/{name}/runs/{id}`: one run with its whole log.
pub async fn run(
    State(s): Tasks,
    Path((name, id)): Path<(String, i64)>,
) -> Result<Json<Value>, ApiError> {
    let run = on_scheduler(s, move |s| Ok(s.writer().state().run(id)?)).await?;
    match run {
        Some(r) if r.task == name => Ok(Json(run_json(&r))),
        _ => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("task {name:?} has no run {id}"),
        )),
    }
}

/// `POST /api/tasks/{name}/run`: run the task now and answer with the run once it ends.
pub async fn trigger(State(s): Tasks, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let run = on_scheduler(s, move |s| s.trigger(&name)).await?;
    Ok(Json(run_json(&run)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronQuery {
    schedule: String,
    timezone: Option<String>,
    count: Option<usize>,
}

/// The most firings one preview lists.
const MAX_PREVIEW: usize = 50;

/// `GET /api/cron?schedule=&timezone=&count=`: a schedule in words and the next times it fires
/// after now, read by the scheduler's own cron parser. A schedule or time zone the scheduler
/// would refuse is a 400 with the same issues a task file with it gets.
pub async fn cron(State(s): Tasks, Query(q): Query<CronQuery>) -> Result<Json<Value>, ApiError> {
    let timezone = q.timezone.unwrap_or_else(|| "UTC".to_owned());
    let count = q.count.unwrap_or(5).clamp(1, MAX_PREVIEW);
    match scheduler::preview(&q.schedule, &timezone, s.now_ms(), count) {
        Ok((text, next)) => Ok(Json(json!({
            "schedule": q.schedule,
            "timezone": timezone,
            "text": text,
            "next": next,
        }))),
        Err(issues) => Err(ApiError::from(writer::Error::Invalid(schema::Invalid {
            file: "schedule".to_owned(),
            issues,
        }))),
    }
}

// ---- secrets --------------------------------------------------------------------------------

type Shared = State<Arc<Writer>>;

fn secret_json(s: &SecretInfo) -> Value {
    json!({ "name": s.name, "created": s.created_ms, "updated": s.updated_ms })
}

fn find_secret(w: &Writer, name: &str) -> Result<SecretInfo, state::Error> {
    w.state()
        .list_secrets()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| state::Error::UnknownSecret(name.to_owned()))
}

/// `GET /api/secrets`: every secret's name and times, never its value.
pub async fn secrets(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let list = blocking(w, |w| w.state().list_secrets()).await?;
    let list: Vec<Value> = list.iter().map(secret_json).collect();
    Ok(Json(json!({ "secrets": list })))
}

/// `GET /api/secrets/{name}`: the secret's name and times, never its value.
pub async fn secret(State(w): Shared, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let info = blocking(w, move |w| find_secret(w, &name)).await?;
    Ok(Json(secret_json(&info)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretBody {
    value: String,
}

/// `PUT /api/secrets/{name}`: store or replace a secret, encrypted at rest.
pub async fn put_secret(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Json<Value>, ApiError> {
    check_name("secret", &name)?;
    let b: SecretBody = body(&headers, &raw, |value| SecretBody { value })?;
    let info = blocking(w, move |w| {
        w.state().set_secret(&name, &b.value)?;
        find_secret(w, &name)
    })
    .await?;
    Ok(Json(secret_json(&info)))
}

/// `DELETE /api/secrets/{name}`.
pub async fn delete_secret(
    State(w): Shared,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    blocking(w, move |w| w.state().delete_secret(&n)).await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

// ---- test clock -----------------------------------------------------------------------------

/// The scheduler and the clock it reads, when the server runs with `--test-clock`.
#[derive(Clone)]
pub struct TestTime {
    pub scheduler: Arc<Scheduler>,
    pub clock: Arc<TestClock>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockBody {
    advance: Option<String>,
    set: Option<String>,
}

/// `GET /api/test/clock`.
pub async fn clock(State(t): State<TestTime>) -> Json<Value> {
    Json(json!({ "now": t.scheduler.now_ms() }))
}

/// `POST /api/test/clock`: move the clock, then run what became due.
pub async fn move_clock(
    State(t): State<TestTime>,
    Json(b): Json<ClockBody>,
) -> Result<Json<Value>, ApiError> {
    match (b.advance, b.set) {
        (Some(by), None) => {
            let by = scheduler::parse_duration(&by).map_err(ApiError::bad_request)?;
            t.clock.advance(by);
        }
        (None, Some(at)) => {
            let at = scheduler::parse_time(&at)
                .ok_or_else(|| ApiError::bad_request(format!("{at:?} is not an RFC 3339 time")))?;
            t.clock.set(at);
        }
        _ => {
            return Err(ApiError::bad_request(
                "send exactly one of `advance` and `set`",
            ));
        }
    }
    let s = t.scheduler.clone();
    let runs = on_scheduler(s, |s| Ok(s.tick())).await?;
    let runs: Vec<Value> = runs.iter().map(run_json).collect();
    Ok(Json(json!({ "now": t.scheduler.now_ms(), "runs": runs })))
}
