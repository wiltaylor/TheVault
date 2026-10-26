//! Agents and the vault's own scripts and tasks: `run_saved_script`, `run_task`, `list_scripts`
//! and `list_tasks`, offered to an agent whose file sets `permissions { scripts { run_saved =
//! true } }` or `run_code = true`, which implies it ([`schema::AgentScripts::runs_saved`]).
//!
//! - `run_saved_script { script, function, args }` calls exported `function` of
//!   `scripts/<script>.ws` with `args`, JSON text of an array, each element a `Value`. It runs
//!   with the modules a task has; `http` reaches any host only when the agent may read web pages
//!   (`permissions.web.read`), and none otherwise; it reads every secret only when the agent may
//!   run its own code (`run_code`), and none otherwise; no `fs` directories; five minutes. Its
//!   changes are made at once, as the agent.
//! - `run_task { name }` starts the task as `POST /api/tasks/{name}/run` does, with the task's
//!   own capabilities and as the task, and waits up to [`TASK_WAIT`] for it to end: then it
//!   answers the run's outcome and log, and otherwise that it started, with the run's id.
//! - `list_scripts` answers each script's exported functions and how many arguments each takes,
//!   or why it does not compile as a task; `list_tasks` each task's schedule and latest run.
//!
//! They run on the home server's script host and scheduler, so like the other script tools they
//! are not offered where there is none, nor to a turn whose scope cannot write notes
//! (`crate::ai_scripts`). Each run's summary carries `script`, the version that ran, `{ kind:
//! "vault", script, hash, generation }` (with `task` for a task), which `crate::ai_activity`
//! records as an `ai.script` entry.

use std::time::{Duration, Instant};

use ai::ToolSpec;
use scheduler::Scheduler;
use schema::Agent;
use script::{ALL_SECRETS, Host, Job, Kind};
use serde_json::{Value, json};
use state::{Author, RunOutcome};
use writer::Writer;

use crate::ai_chat::{ToolOut, tool};
use crate::ai_scripts::{CODE_TIMEOUT, parse_args};
use crate::script_tools::{Ran, hash};

/// The tool that runs a vault script's function.
pub const RUN_SAVED: &str = "run_saved_script";

/// The tool that starts a scheduled task now.
pub const RUN_TASK: &str = "run_task";

/// The tools that list what the other two run.
pub const LIST_SCRIPTS: &str = "list_scripts";
pub const LIST_TASKS: &str = "list_tasks";

/// How long `run_task` waits for the task to end before answering that it started.
pub const TASK_WAIT: Duration = Duration::from_secs(60);

/// [`TASK_WAIT`], or in a test build the milliseconds `VAULT_TEST_RUN_TASK_WAIT_MS` names.
fn task_wait() -> Duration {
    if cfg!(debug_assertions)
        && let Some(ms) = std::env::var("VAULT_TEST_RUN_TASK_WAIT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    TASK_WAIT
}

/// The four tools, in the order they are offered.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        tool(
            LIST_SCRIPTS,
            "List the vault's saved scripts (scripts/<name>.ws): each script's name and the functions it exports with how many arguments each takes, or why it does not compile. Call one with run_saved_script.",
            json!({}),
        ),
        tool(
            RUN_SAVED,
            "Run an exported function of one of the vault's saved scripts (scripts/<script>.ws) with `args`, each element arriving as a `Value` parameter. It runs with the vault, value, json, math, regex, http, ai and secrets modules for up to 5 minutes: http reaches any host only when you may read web pages, and secrets only when you may run your own code. Changes are made at once, as you, and recorded in history. Answers the function's return value as JSON, its log, the time it took and the instructions it used, or the compile or run-time error.",
            json!({
                "script": { "type": "string", "description": "The script's name, as list_scripts answers it." },
                "function": { "type": "string", "description": "The exported function to call." },
                "args": { "type": ["string", "null"], "description": "JSON text of an array, one element per argument of the function, each a Value; null for none." },
            }),
        ),
        tool(
            LIST_TASKS,
            "List the vault's scheduled tasks (tasks/<name>.wcl): each task's name, label, schedule in words, script and function, whether it is paused or running, when it runs next, and how its latest run went. Start one now with run_task.",
            json!({}),
        ),
        tool(
            RUN_TASK,
            "Start one of the vault's scheduled tasks now, paused or not, as its Run now button does: it runs with its own hosts, secrets and folders, and its changes are made as the task. Waits up to 60 seconds for it to end and answers its outcome and log; a task still running then answers started with its run id, and list_tasks reads how it went later.",
            json!({ "name": { "type": "string", "description": "The task's name, as list_tasks answers it." } }),
        ),
    ]
}

/// The script reference of a run of `script`'s `source`, read at `generation`.
fn reference(script: &str, source: Option<&str>, generation: Option<u64>) -> Value {
    json!({
        "kind": "vault",
        "script": script,
        "hash": source.map(hash),
        "generation": generation,
    })
}

/// The job of `run_saved_script` call `input` by `agent` on `w`'s vault and its script
/// reference. A call that cannot run is the refusal.
pub(crate) fn saved_job(w: &Writer, agent: &Agent, input: &Value) -> Result<(Job, Value), Ran> {
    let name = input["script"].as_str().unwrap_or_default().trim();
    let function = input["function"].as_str().unwrap_or_default().trim();
    let blank = reference(name, None, None);
    if !schema::is_name(name) {
        return Err(Ran::refused(
            "arguments",
            format!("{name:?} is not a script name: list_scripts answers them"),
            blank,
        ));
    }
    if function.is_empty() {
        return Err(Ran::refused(
            "arguments",
            "function must name an exported function of the script".into(),
            blank,
        ));
    }
    let (source, generation) = w
        .script_snapshot(name)
        .map_err(|e| Ran::refused("not_found", e.to_string(), blank.clone()))?;
    let script = reference(name, Some(&source), Some(generation));
    let args = parse_args(&input["args"], &script)?;
    let p = &agent.permissions;
    let mut job = Job::new(Kind::Task, name, &source, function);
    job.args = args;
    job.allow_domains = if p.web.read {
        vec!["*".to_owned()]
    } else {
        Vec::new()
    };
    job.secrets = if p.scripts.run_code {
        vec![ALL_SECRETS.to_owned()]
    } else {
        Vec::new()
    };
    job.author = Some(Author::Agent(agent.name.clone()));
    job.timeout = Some(CODE_TIMEOUT);
    Ok((job, script))
}

/// `run_saved_script`: blocks until the script ends.
pub(crate) fn run_saved(w: &Writer, host: &Host, agent: &Agent, input: &Value) -> Ran {
    match saved_job(w, agent, input) {
        Ok((job, script)) => crate::script_tools::run(host, job, script),
        Err(refused) => refused,
    }
}

/// How a `run_task` call went: the run, and whether it was still running when the wait ended.
pub(crate) struct TaskRan {
    pub ran: Ran,
    pub started: bool,
    pub run_id: Option<i64>,
}

/// `run_task`: start the task as `POST /api/tasks/{name}/run` does and wait up to
/// [`task_wait`] for it to end.
pub(crate) fn run_task(s: &std::sync::Arc<Scheduler>, input: &Value) -> TaskRan {
    let name = input["name"].as_str().unwrap_or_default().trim().to_owned();
    let refused = |kind, message: String, script: Value| TaskRan {
        ran: Ran::refused(kind, message, script),
        started: false,
        run_id: None,
    };
    let mut script =
        json!({ "kind": "vault", "task": name, "script": null, "hash": null, "generation": null });
    let def = match s.status(&name) {
        Ok(st) => st.def,
        Err(e) => return refused("not_found", e.to_string(), script),
    };
    let def = match def {
        Ok(d) => d,
        Err(message) => {
            return refused(
                "invalid",
                format!("the task {name} cannot run: {message}"),
                script,
            );
        }
    };
    script["script"] = json!(def.script);
    if let Ok((source, generation)) = s.writer().script_snapshot(&def.script) {
        script["hash"] = json!(hash(&source));
        script["generation"] = json!(generation);
    }
    let began = Instant::now();
    let run = match s.trigger_in_background(&name) {
        Ok(r) => r,
        Err(e) => {
            let kind = match e {
                scheduler::Error::Running(_) => "running",
                scheduler::Error::NoTask(_) => "not_found",
                scheduler::Error::Broken { .. } => "invalid",
                _ => "fault",
            };
            return refused(kind, e.to_string(), script);
        }
    };
    let wait = task_wait();
    let mut latest = run;
    let mut pause = Duration::from_millis(20);
    while matches!(latest.outcome, RunOutcome::Running) && began.elapsed() < wait {
        std::thread::sleep(pause.min(wait.saturating_sub(began.elapsed())));
        pause = (pause * 2).min(Duration::from_millis(250));
        match s.writer().state().run(latest.id) {
            Ok(Some(r)) => latest = r,
            Ok(None) => break,
            Err(e) => {
                eprintln!("run_task: the run of {name} could not be read: {e}");
                break;
            }
        }
    }
    let ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (result, started) = match &latest.outcome {
        RunOutcome::Running => (
            Ok(json!({
                "started": true,
                "task": name,
                "run_id": latest.id,
                "message": format!(
                    "The task is still running after {} seconds. It goes on without you; list_tasks answers how its latest run went.",
                    wait.as_secs()
                ),
            })),
            true,
        ),
        RunOutcome::Succeeded => (
            Ok(json!({ "task": name, "run_id": latest.id, "outcome": "succeeded" })),
            false,
        ),
        RunOutcome::Failed(m) => (Err(("task_failed", m.clone())), false),
    };
    TaskRan {
        ran: Ran {
            result,
            log: latest.log.clone(),
            ms,
            instructions: 0,
            script,
        },
        started,
        run_id: Some(latest.id),
    }
}

/// `list_scripts`: every script with its exported functions, checked on `host` as a task.
pub(crate) fn list_scripts(w: &Writer, host: &Host) -> ToolOut {
    let names = match w.scripts() {
        Ok(n) => n,
        Err(e) => return ToolOut::error(e.to_string()),
    };
    let scripts: Vec<Value> = names
        .iter()
        .map(|name| {
            let source = w.read_script(name).unwrap_or_default();
            match host.exports(Kind::Task, name, &source) {
                Ok(fns) => json!({
                    "name": name,
                    "functions": fns
                        .iter()
                        .map(|(f, n)| json!({ "name": f, "args": n }))
                        .collect::<Vec<_>>(),
                    "problem": null,
                }),
                Err(e) => json!({ "name": name, "functions": [], "problem": e.to_string() }),
            }
        })
        .collect();
    ToolOut {
        content: json!({ "scripts": scripts }).to_string(),
        is_error: false,
        summary: json!({ "notes": [] }),
        proposal: None,
    }
}

/// `list_tasks`: every task with its schedule and latest run, without the run's log.
pub(crate) fn list_tasks(s: &Scheduler) -> ToolOut {
    let all = match s.statuses() {
        Ok(a) => a,
        Err(e) => return ToolOut::error(e.to_string()),
    };
    let tasks: Vec<Value> = all
        .iter()
        .map(|t| {
            let latest = t.latest_run.as_ref().map(|r| {
                let mut v = crate::tasks::run_json(r);
                if let Some(o) = v.as_object_mut() {
                    o.remove("log");
                }
                v
            });
            let mut v = json!({
                "name": t.name,
                "running": t.running,
                "next_run": t.next_run_ms,
                "latest_run": latest,
            });
            match &t.def {
                Ok(d) => {
                    v["label"] = json!(d.label);
                    v["description"] = json!(d.description);
                    v["schedule"] = json!(d.schedule);
                    v["schedule_text"] = json!(d.schedule_text());
                    v["timezone"] = json!(d.timezone.name());
                    v["script"] = json!(d.script);
                    v["function"] = json!(d.function);
                    v["paused"] = json!(d.paused);
                    v["timeout_ms"] = json!(d.timeout.as_millis() as u64);
                }
                Err(m) => v["error"] = json!(m),
            }
            v
        })
        .collect();
    ToolOut {
        content: json!({ "tasks": tasks }).to_string(),
        is_error: false,
        summary: json!({ "notes": [] }),
        proposal: None,
    }
}
