//! Scheduled tasks, their run history, and the secrets they read (D10).
//!
//! - `PUT /api/tasks/{name}` takes the task file as WCL text, or JSON `{ "wcl": "..." }`. It is
//!   checked against `docs/vault-format.md` "Task files", and the script it names must exist.
//! - `POST /api/tasks/{name}/run` starts the task now, paused or not, and answers 202 with the
//!   run as soon as it starts; `?wait=true` answers 200 with the run once it ends instead. A
//!   failure schedules retries as a scheduled run's does.
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
use crate::{ApiError, Caller, blocking};

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
    Caller(by): Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: TaskBody = body(&headers, &raw, |wcl| TaskBody { wcl })?;
    let (status, wcl) = on_scheduler(s, move |s| {
        let _acting = writer::acting_as(&by);
        let wcl = s.put_task(&name, &b.wcl)?;
        Ok((s.status(&name)?, wcl))
    })
    .await?;
    let mut json = task_json(&status);
    json["wcl"] = json!(wcl);
    Ok(text_or_json(&headers, wcl, json))
}

/// `DELETE /api/tasks/{name}`. The task's run history stays.
pub async fn delete(
    State(s): Tasks,
    Caller(by): Caller,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    on_scheduler(s, move |s| {
        let _acting = writer::acting_as(&by);
        s.delete_task(&n)
    })
    .await?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerQuery {
    #[serde(default)]
    wait: bool,
}

/// `POST /api/tasks/{name}/run`: start the task now and answer 202 with the run, still
/// `running`, as soon as it is recorded; `GET /api/tasks/{name}/runs/{id}` reads it as it goes.
/// With `?wait=true`, answer 200 with the run once it ends.
pub async fn trigger(
    State(s): Tasks,
    Path(name): Path<String>,
    Query(q): Query<TriggerQuery>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if q.wait {
        let run = on_scheduler(s, move |s| s.trigger(&name)).await?;
        return Ok((StatusCode::OK, Json(run_json(&run))));
    }
    let run = tokio::task::spawn_blocking(move || s.trigger_in_background(&name))
        .await
        .map_err(|e| {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })??;
    Ok((StatusCode::ACCEPTED, Json(run_json(&run))))
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

fn login_json(s: &state::LoginInfo) -> Value {
    json!({"name":s.name,"kind":"login","created":s.created_ms,"updated":s.updated_ms,
        "username":s.username,"origins":s.origins,"has_totp":s.has_totp})
}

fn find_secret_json(w: &Writer, name: &str) -> Result<Value, ApiError> {
    if w.state().is_login_secret(name)? {
        let info = w
            .state()
            .list_login_secrets()?
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| state::Error::UnknownSecret(name.to_owned()))?;
        return Ok(login_json(&info));
    }
    Ok(secret_json(&find_secret(w, name)?))
}

/// Login secrets are managed only by sessions or stored user-kind admin tokens.
pub(crate) fn may_manage_login_secrets(
    w: &Writer,
    author: &state::Author,
) -> Result<bool, state::Error> {
    Ok(match author {
        state::Author::Session(_) => true,
        state::Author::Token(name) => {
            w.state().device_for_token(name)?.is_none()
                && w.state()
                    .list_tokens()?
                    .iter()
                    .any(|t| t.name == *name && t.kind == "user" && t.scope == state::Scope::Admin)
        }
        _ => false,
    })
}

fn require_login_owner(w: &Writer, author: &state::Author) -> Result<(), ApiError> {
    if may_manage_login_secrets(w, author)? {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "Login secrets require an owner session or admin user token",
    ))
}

fn validate_login(b: SecretBody) -> Result<state::LoginSecret, ApiError> {
    let invalid =
        || ApiError::bad_request("invalid Login fields, password, base32 seed or site origins");
    if b.value.is_some() {
        return Err(invalid());
    }
    let password = b.password.filter(|p| !p.is_empty()).ok_or_else(invalid)?;
    let username = b.username.ok_or_else(invalid)?;
    let seed = b
        .totp_seed
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_owned());
    if let Some(seed) = &seed
        && crate::browser_redaction::totp(seed, 0, 6, 30).is_none()
    {
        return Err(ApiError::bad_request(
            "Invalid one-time-code seed: use a base32 key or otpauth://totp URI with SHA1, SHA256 or SHA512, digits 6-8 and a positive period.",
        ));
    }
    let mut origins = Vec::new();
    for origin in b.origins.unwrap_or_default() {
        let canonical = state::canonical_origin(&origin).ok_or_else(invalid)?;
        if !origins.contains(&canonical) {
            origins.push(canonical);
        }
    }
    Ok(state::LoginSecret {
        username,
        password,
        totp_seed: seed,
        origins,
    })
}

fn find_secret(w: &Writer, name: &str) -> Result<SecretInfo, state::Error> {
    w.state()
        .list_secrets()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| state::Error::UnknownSecret(name.to_owned()))
}

/// `GET /api/secrets`: every secret's name and times, never its value.
pub async fn secrets(
    State(w): Shared,
    crate::auth::Caller(by): crate::auth::Caller,
) -> Result<Json<Value>, ApiError> {
    let list = blocking(w, move |w| -> Result<_, ApiError> {
        let mut list: Vec<Value> = w.state().list_secrets()?.iter().map(secret_json).collect();
        if may_manage_login_secrets(w, &by)? {
            list.extend(w.state().list_login_secrets()?.iter().map(login_json));
        }
        list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(list)
    })
    .await?;
    Ok(Json(json!({ "secrets": list })))
}

/// `GET /api/secrets/{name}`: the secret's name and times, never its value.
pub async fn secret(State(w): Shared, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        blocking(w, move |w| find_secret_json(w, &name)).await?,
    ))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SecretBody {
    value: Option<String>,
    kind: Option<String>,
    username: Option<String>,
    password: Option<String>,
    totp_seed: Option<String>,
    origins: Option<Vec<String>>,
}

/// Internal state must go through its dedicated API, for every caller and secret kind.
fn require_user_secret(name: &str) -> Result<(), ApiError> {
    if !state::reserved_secret(name) {
        return Ok(());
    }
    let destination = if name.starts_with("ai_") {
        "Settings > AI (the accounts API)"
    } else if name.starts_with("google_") {
        "Settings > Google"
    } else if name.starts_with("gitsync_") {
        "Settings > Git Sync"
    } else {
        "Settings > Brave Search"
    };
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        "internal_secret",
        format!("This secret is managed by the server. Use {destination}, not /api/secrets."),
    ))
}

/// `PUT /api/secrets/{name}`: store or replace a user secret, encrypted at rest.
pub async fn put_secret(
    State(w): Shared,
    Path(name): Path<String>,
    Caller(author): Caller,
    seams: Option<axum::Extension<crate::TestSecretSeams>>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Json<Value>, ApiError> {
    check_name("secret", &name)?;
    if !test_seam_allowed(&name, &author, seams.is_some()) {
        require_user_secret(&name)?;
    }
    let b: SecretBody = body(&headers, &raw, |value| SecretBody {
        value: Some(value),
        ..SecretBody::default()
    })
    .map_err(|_| ApiError::bad_request("invalid secret body"))?;
    let info = blocking(w, move |w| {
        let login = b.kind.as_deref() == Some("login");
        if login || w.state().is_login_secret(&name)? {
            require_login_owner(w, &author)?;
        }
        if login {
            if w.state().list_secrets()?.iter().any(|s| s.name == name) {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "secret_kind",
                    "name belongs to a string secret",
                ));
            }
            w.state().set_login_secret(&name, &validate_login(b)?)?;
        } else {
            if w.state().is_login_secret(&name)? {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "secret_kind",
                    "name belongs to a Login secret",
                ));
            }
            if b.kind.as_deref().is_some_and(|k| k != "string")
                || b.username.is_some()
                || b.password.is_some()
                || b.totp_seed.is_some()
                || b.origins.is_some()
            {
                return Err(ApiError::bad_request("invalid string secret fields"));
            }
            w.state().set_secret(
                &name,
                &b.value
                    .ok_or_else(|| ApiError::bad_request("value required"))?,
            )?;
        }
        find_secret_json(w, &name)
    })
    .await?;
    Ok(Json(info))
}

/// `DELETE /api/secrets/{name}`.
pub async fn delete_secret(
    State(w): Shared,
    Path(name): Path<String>,
    Caller(author): Caller,
    seams: Option<axum::Extension<crate::TestSecretSeams>>,
) -> Result<Json<Value>, ApiError> {
    if !name.starts_with(state::GITSYNC_FRESHNESS_RECEIVE_PREFIX)
        && !test_seam_allowed(&name, &author, seams.is_some())
    {
        require_user_secret(&name)?;
    }
    let n = name.clone();
    blocking(w, move |w| -> Result<(), ApiError> {
        if w.state().is_login_secret(&n)? {
            require_login_owner(w, &author)?;
            w.state().delete_login_secret(&n)?;
        } else {
            w.state().delete_secret(&n)?;
        }
        Ok(())
    })
    .await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

fn test_seam_allowed(name: &str, author: &state::Author, enabled: bool) -> bool {
    cfg!(debug_assertions)
        && enabled
        && matches!(author, state::Author::Session(_))
        && state::test_seam_secret(name)
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
