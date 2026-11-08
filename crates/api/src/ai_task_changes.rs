//! Agents and the vault's scheduled tasks, `tasks/<name>.wcl` (TV-71, S3 of TV-32).
//!
//! - `get_task { name }` reads one task: its file, the task as the scheduler reads it, when it
//!   runs next, the source of its script, and what else runs that script.
//! - `propose_task_change { name, wcl, script_source }` and `propose_task_delete { name }` check a
//!   change and hand it to the user as a `task_proposal`, `kind` always `task`. A change is checked
//!   as `PUT /api/tasks/{name}` checks it (`Scheduler::check_task`), without writing: the file,
//!   the script it names (which must exist, or be `script_source`), the script compiled as a task
//!   with its function taking no arguments, and `script_source` compiled in every other way the
//!   script runs. The proposal always waits for the user's Apply, whatever the agent's switches
//!   say: both tools are in `crate::ai_chat`'s `ALWAYS_PROPOSE`, so a trigger's run is not offered
//!   them and `Direct::covers` never covers a `task_proposal` (TV-32 Q2 O1). `POST /api/ai/apply`
//!   applies it through [`apply_task`], with `admin` or as the copy that waits in the thread.
//!
//! An agent never proposes `read_paths` or `write_paths` (TV-32 Q3): the scheduler refuses only a
//! path inside the vault, so a path such as `/` would reach the secret key file and the host. A
//! proposal whose file has either is refused, here and again at Apply; the user adds paths in the
//! task editor. Nor does an agent change the code such a task runs: a `script_source` for a script
//! that another task with `fs` paths runs is refused too, naming that task (TV-71 review F1). The
//! card shows the script, written or not, and Apply stale-checks it either way. A task that may
//! reach any host (`allow_domains = ["*"]`) is allowed, with a warning
//! (TV-32 Q4), as are secrets with hosts, with a warning that the task could send them out
//! (TV-32 R4). A new task runs on its schedule once applied, and the card shows its next three
//! runs (TV-32 Q5). A delete keeps the task's run history.
//!
//! The tools need the home server's scheduler (`crate::script_tools::scheduler_for`); elsewhere
//! they answer an error.

use std::sync::Arc;

use ai::ToolSpec;
use scheduler::{Scheduler, TaskDef};
use schema::Agent;
use script::Kind;
use serde_json::{Value, json};
use writer::Writer;

use crate::ai_chat::{ApplyError, ToolOut, str_arg, tool};
use crate::ai_script_changes::{any_host_warning, described, shared_paths_refused};

/// The tools that change a scheduled task. Their change always waits for the user's Apply.
pub const PROPOSE_TOOLS: [&str; 2] = ["propose_task_change", "propose_task_delete"];

/// How many next runs a proposal names.
const NEXT_RUNS: usize = 3;

/// The text the model reads before writing a task file.
const FORMAT: &str = "A task is a file tasks/<name>.wcl, <name> a lowercase identifier ([a-z][a-z0-9_]*), holding fields only: label (string), description (string), schedule (required: a five-field cron expression, minute hour day-of-month month day-of-week), timezone (an IANA time zone, default \"UTC\"), script (required: the name of scripts/<script>.ws), function (the exported function to call, default \"run\", which takes no arguments), allow_domains (list of host names the http module may reach, [\"*\"] any host, including the local network), secrets (list of secret names it may read), retries (0 to 20, default 3), retry_delay (ISO 8601 duration, default \"PT1M\", doubling each retry), timeout (ISO 8601 duration, default \"PT30S\"), paused (bool, default false). Agents never propose read_paths or write_paths: only the user sets those, in the task editor. The script runs with the vault, value, json, math, regex, http, secrets and fs modules, and its changes are made as the task.";

/// The read and propose tools, in every agent's list like any other.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool(
            "get_task",
            "Read one of the vault's scheduled tasks (tasks/<name>.wcl): its whole file, the task as the scheduler reads it, its schedule in words and next runs, the source of the script it names, and what else runs that script. Also answers the task-file vocabulary. Read it before proposing a change.",
            json!({ "name": { "type": "string", "description": "The task's name." } }),
        ),
        tool(
            "propose_task_change",
            "Propose a scheduled task for the user to apply: a new task under a new name, or the whole new file of an existing one, and, when given, the whole source of the script it names, written together. It is checked as saving the task would check it, and its script must compile as a task, with its function taking no arguments. Never give read_paths or write_paths: a task file with either is refused, and only the user adds them. The change always waits for the user, who sees the file and script diff, the schedule in words with the next runs, the hosts and secrets, and applies or discards it. Once applied, a task that is not paused runs on its schedule.",
            json!({
                "name": { "type": "string", "description": "The task's name: an existing task to change, or a new name ([a-z][a-z0-9_]*) to create one." },
                "wcl": { "type": "string", "description": "The whole task file as WCL text." },
                "script_source": { "type": ["string", "null"], "description": "The whole wscript source of the script the task names, written as scripts/<script>.ws; null to leave the script as it is (it must exist then)." },
            }),
        ),
        tool(
            "propose_task_delete",
            "Propose deleting a scheduled task for the user to apply. Its script stays, and so does its run history. The delete always waits for the user.",
            json!({ "name": { "type": "string", "description": "The task's name." } }),
        ),
    ]
}

fn out(content: Value, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [] }),
        proposal: proposal.map(|p| ("task_proposal", p)),
    }
}

/// The error with `content` as the model reads it.
fn refused(message: String, mut content: Value) -> ToolOut {
    content["error"] = json!(message);
    let mut o = ToolOut::error(message);
    o.content = content.to_string();
    o
}

/// The home server's scheduler, or the error for a server without one.
fn scheduler(w: &Writer) -> Result<Arc<Scheduler>, ToolOut> {
    crate::script_tools::scheduler_for(w).ok_or_else(|| {
        ToolOut::error(
            "scheduled tasks are changed on the home server only, and this server runs none",
        )
    })
}

/// A task name, checked.
fn name_arg(input: &Value) -> Result<&str, ToolOut> {
    let name = str_arg(input, "name")?;
    if schema::is_name(name) {
        Ok(name)
    } else {
        Err(ToolOut::error(format!(
            "{name:?} is not a valid task name: use [a-z][a-z0-9_]*, at most 64 characters"
        )))
    }
}

/// The error for a task that does not exist, naming those that do.
fn missing(w: &Writer, name: &str) -> ToolOut {
    let names: Vec<String> = w
        .task_files()
        .unwrap_or_default()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    ToolOut::error(if names.is_empty() {
        format!("there is no task {name:?}: the vault has no scheduled tasks")
    } else {
        format!(
            "there is no task {name:?}; the vault's tasks are {}",
            names.join(", ")
        )
    })
}

/// The task as the card and the model read it.
fn task_json(def: &TaskDef) -> Value {
    json!({
        "label": def.label,
        "description": def.description,
        "schedule": def.schedule,
        "schedule_text": def.schedule_text(),
        "timezone": def.timezone.name(),
        "script": def.script,
        "function": def.function,
        "allow_domains": def.allow_domains,
        "secrets": def.secrets,
        "read_paths": def.read_paths,
        "write_paths": def.write_paths,
        "retries": def.retries,
        "retry_delay_ms": def.retry_delay.as_millis() as u64,
        "timeout_ms": def.timeout.as_millis() as u64,
        "paused": def.paused,
    })
}

/// The next runs of `def` after `now`: none while it is paused.
fn next_runs(def: &TaskDef, now: i64) -> Vec<i64> {
    if def.paused {
        Vec::new()
    } else {
        def.next_runs(now, NEXT_RUNS)
    }
}

/// What else runs the script of task `name`, the task itself left out, and the warnings for it.
fn others(w: &Writer, script: &str, name: &str) -> (Value, Vec<String>) {
    let uses: Vec<_> = w
        .script_runs_as(script, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|u| !(u.kind == "task" && u.name == name))
        .collect();
    described(&uses)
}

/// `", "`-joined, for a warning.
fn listed(items: &[String]) -> String {
    items.join(", ")
}

/// The warnings for what task `def` may reach (TV-32 Q4, R4): any host; secrets that may be sent
/// to the hosts it reaches; secrets that do not exist yet.
fn grant_warnings(w: &Writer, def: &TaskDef) -> Vec<String> {
    let mut out = Vec::new();
    let what = format!("The task {}", def.name);
    if def.allow_domains.iter().any(|d| d == "*") {
        out.push(any_host_warning(&what));
    }
    if !def.secrets.is_empty() && !def.allow_domains.is_empty() {
        out.push(format!(
            "{what} may read the secrets {} and reach {}: it could send those secrets there.",
            listed(&def.secrets),
            if def.allow_domains.iter().any(|d| d == "*") {
                "any host".to_owned()
            } else {
                listed(&def.allow_domains)
            }
        ));
    }
    let known: Vec<String> = w
        .state()
        .list_secrets()
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.name)
        .collect();
    let unknown: Vec<String> = def
        .secrets
        .iter()
        .filter(|s| !known.contains(s))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        out.push(format!(
            "There is no secret {} yet: add it in Settings before the task runs.",
            listed(&unknown)
        ));
    }
    out
}

/// The refusal of `read_paths` or `write_paths` in a task file from an agent (TV-32 Q3).
fn paths_refused(def: &TaskDef, current_has_paths: bool) -> Option<String> {
    if def.read_paths.is_empty() && def.write_paths.is_empty() {
        return None;
    }
    Some(if current_has_paths {
        format!(
            "task {:?} has read_paths or write_paths, which only the user changes: an agent may not propose a task file with them. Tell the user the change to make in the task editor",
            def.name
        )
    } else {
        format!(
            "an agent may not propose read_paths or write_paths for task {:?}: leave them out, and tell the user which folders the task needs, to add in the task editor",
            def.name
        )
    })
}

/// Whether the text of a task file names `fs` paths.
fn has_paths(name: &str, text: &str) -> bool {
    TaskDef::parse(name, text).is_ok_and(|d| !d.read_paths.is_empty() || !d.write_paths.is_empty())
}

/// `get_task`.
pub(crate) fn get(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = name_arg(input)?;
    let s = scheduler(w)?;
    let wcl = w.read_task(name).map_err(|_| missing(w, name))?;
    let mut content = json!({ "name": name, "wcl": wcl, "format": FORMAT });
    match TaskDef::parse(name, &wcl) {
        Ok(def) => {
            let (users, warnings) = others(w, &def.script, name);
            content["task"] = task_json(&def);
            content["next_runs"] = json!(next_runs(&def, s.now_ms()));
            content["script_source"] = json!(w.read_script(&def.script).ok());
            content["runs_as"] = users;
            content["warnings"] = json!(warnings);
        }
        Err(inv) => content["problems"] = json!(inv.to_string()),
    }
    Ok(out(content, None))
}

/// Why `source` would not run as the script of task `def`: compiled as a task with its function
/// taking no arguments, and, when it is a new source, in every other way the script runs.
fn script_problems(
    w: &Writer,
    s: &Scheduler,
    def: &TaskDef,
    source: &str,
    new_source: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    match s
        .host()
        .entry(Kind::Task, &def.script, source, &def.function)
    {
        Ok(0) => {}
        Ok(n) => problems.push(format!(
            "function {:?} of script {:?} takes {n} arguments: a task's function takes none",
            def.function, def.script
        )),
        Err(e) => problems.push(e.to_string()),
    }
    if new_source {
        match w.script_problems(&def.script, source) {
            Ok(found) => {
                for p in found {
                    if !problems.contains(&p.message) {
                        problems.push(p.message);
                    }
                }
            }
            Err(e) => problems.push(e.to_string()),
        }
    }
    problems
}

/// `propose_task_change`: check the task file and its script as saving them would, without
/// writing, and hand them to the user with the schedule, the next runs and the grants.
pub(crate) fn propose_change(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = name_arg(input)?;
    let wcl = input["wcl"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolOut::error("wcl must be the whole task file as WCL text"))?;
    let s = scheduler(w)?;
    let script_given = input["script_source"].as_str();
    let (after, def) = s.check_task(name, wcl, script_given).map_err(|e| {
        let message = e.to_string();
        let issues = crate::ApiError::from(e).body;
        refused(
            message,
            json!({
                "issues": issues,
                "message": "The task was not proposed. Fix the issues and propose it again; get_task answers the task-file vocabulary.",
            }),
        )
    })?;
    let before = w.read_task(name).ok();
    if let Some(why) = paths_refused(&def, before.as_deref().is_some_and(|b| has_paths(name, b))) {
        return Err(refused(
            why,
            json!({ "message": "The task was not proposed." }),
        ));
    }
    let script_before = w.read_script(&def.script).ok();
    let script_after = script_given.filter(|src| Some(*src) != script_before.as_deref());
    if before.as_deref() == Some(after.as_str()) && script_after.is_none() {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed task and script are the same as the current ones: there is nothing to apply.",
            }),
            None,
        ));
    }
    if script_after.is_some()
        && let Some(why) = shared_paths_refused(w, &def.script, Some(name))
    {
        return Err(refused(
            why,
            json!({ "message": "The task was not proposed." }),
        ));
    }
    if let Some(source) = script_after
        && let Some(why) = crate::ai_script_changes::device_command_refused(&def.script, script_before.as_deref(), source)
    {
        return Err(refused(why, json!({ "message": "The task was not proposed." })));
    }
    let source = script_after
        .or(script_before.as_deref())
        .unwrap_or_default();
    let problems = script_problems(w, &s, &def, source, script_after.is_some());
    if !problems.is_empty() {
        return Err(refused(
            format!("script {:?} would not run as task {name:?}", def.script),
            json!({
                "problems": problems,
                "message": "The task was not proposed. Fix the problems and propose it again.",
            }),
        ));
    }
    let (runs_as, script_warnings) = others(w, &def.script, name);
    let mut warnings = grant_warnings(w, &def);
    if script_after.is_some() {
        // A new source changes every other run of the script, with their grants (TV-32 R5).
        warnings.extend(script_warnings);
    }
    let exists = before.is_some();
    let next = next_runs(&def, s.now_ms());
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": exists,
        "schedule_text": def.schedule_text(),
        "next_runs": next,
        "paused": def.paused,
        "runs_as": runs_as,
        "warnings": warnings,
        "message": "The task is valid and is shown to the user as a proposal, with its schedule, next runs, hosts and secrets. It has not been written: it awaits the user, who applies or discards it. Once applied, it runs on its schedule unless it is paused.",
    });
    let proposal = json!({
        "id": call_id,
        "kind": "task",
        "op": if exists { "change" } else { "create" },
        "name": name,
        "exists": exists,
        "before": before,
        "after": after,
        "script": def.script,
        "script_exists": script_before.is_some(),
        "script_before": script_before,
        "script_after": script_after,
        "task": task_json(&def),
        "next_runs": next,
        "runs_as": runs_as,
        "warnings": warnings,
        "proposer": by.name,
    });
    Ok(out(content, Some(proposal)))
}

/// `propose_task_delete`: the task must exist. Its script and run history stay.
pub(crate) fn propose_delete(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = name_arg(input)?;
    scheduler(w)?;
    let before = w.read_task(name).map_err(|_| missing(w, name))?;
    let def = TaskDef::parse(name, &before).ok();
    let proposal = json!({
        "id": call_id,
        "kind": "task",
        "op": "delete",
        "name": name,
        "exists": true,
        "before": before,
        "after": null,
        "script": def.as_ref().map(|d| d.script.clone()),
        "script_exists": def.as_ref().is_some_and(|d| w.read_script(&d.script).is_ok()),
        "script_before": null,
        "script_after": null,
        "task": def.as_ref().map(task_json),
        "next_runs": [],
        "runs_as": [],
        "warnings": [],
        "proposer": by.name,
    });
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it. The task's script and run history stay.",
        }),
        Some(proposal),
    ))
}

/// A scheduler error as an apply error.
fn apply_error(e: scheduler::Error) -> ApplyError {
    match e {
        scheduler::Error::Writer(e) => ApplyError::Write(e),
        e => ApplyError::Invalid(e.to_string()),
    }
}

/// Apply a `task_proposal` through the scheduler, which reloads. Answers `{op, task_name,
/// script_name, next_run}`, `op` `task_create`, `task_change` or `task_delete`, `script_name` the
/// script written with the task, if any. Every check runs again under the writer's write lock, in
/// one `Writer::commit_checked` with the write (TV-71 review F2, as TV-85 does for scripts): the
/// task file and the script must still be the proposal's `before` and `script_before` (the script
/// whether or not the proposal writes it, as the card showed it), the task must check, its script
/// must still compile as the task runs it, and what else runs the script must be as the card
/// showed it. A task file with `fs` paths is refused, and so is a new script source that another
/// run with `fs` paths shares (TV-32 Q3, TV-71 review F1).
pub(crate) fn apply_task(w: &Writer, p: &Value) -> Result<Value, ApplyError> {
    let name = p["name"]
        .as_str()
        .filter(|s| schema::is_name(s))
        .ok_or_else(|| ApplyError::Invalid("the change has no task name".into()))?;
    let s = crate::script_tools::scheduler_for(w).ok_or_else(|| {
        ApplyError::Invalid("scheduled tasks are changed on the home server only".into())
    })?;
    let stale = |what: &str| {
        ApplyError::Stale(format!(
            "{what} since the proposal, so nothing was written: propose it again"
        ))
    };
    let before = p["before"].as_str();
    if p["op"] == "delete" {
        w.commit_checked(|w| -> Result<Vec<store::Change>, ApplyError> {
            if w.read_task(name).ok().as_deref() != before {
                return Err(stale(&format!("the task {name} changed")));
            }
            Ok(w.task_delete_changes(name)?)
        })?;
        s.reload().map_err(apply_error)?;
        return Ok(
            json!({ "op": "task_delete", "task_name": name, "script_name": null, "next_run": null }),
        );
    }
    let after = p["after"]
        .as_str()
        .ok_or_else(|| ApplyError::Invalid("the change has no after".into()))?;
    let script_after = p["script_after"].as_str();
    let mut script = String::new();
    w.commit_checked(|w| -> Result<Vec<store::Change>, ApplyError> {
        if w.read_task(name).ok().as_deref() != before {
            return Err(stale(&format!("the task {name} changed")));
        }
        let (def, changes) = s
            .task_changes(w, name, after, script_after)
            .map_err(apply_error)?;
        if let Some(why) = paths_refused(&def, false) {
            return Err(ApplyError::Refused(why));
        }
        let script_before = w.read_script(&def.script).ok();
        if script_before.as_deref() != p["script_before"].as_str() {
            return Err(stale(&format!("the script {} changed", def.script)));
        }
        if script_after.is_some()
            && let Some(why) = shared_paths_refused(w, &def.script, Some(name))
        {
            return Err(ApplyError::Refused(why));
        }
        if let Some(source) = script_after
            && let Some(why) = crate::ai_script_changes::device_command_refused(&def.script, script_before.as_deref(), source)
        {
            return Err(ApplyError::Refused(why));
        }
        let source = script_after
            .or(script_before.as_deref())
            .unwrap_or_default();
        if !script_problems(w, &s, &def, source, script_after.is_some()).is_empty() {
            return Err(stale(&format!(
                "the script {} no longer runs as this task",
                def.script
            )));
        }
        if others(w, &def.script, name).0 != p["runs_as"] {
            return Err(stale(&format!(
                "what runs the script {} changed",
                def.script
            )));
        }
        script = def.script;
        Ok(changes)
    })?;
    s.reload().map_err(apply_error)?;
    let op = if before.is_some() {
        "task_change"
    } else {
        "task_create"
    };
    let next_run = s.status(name).ok().and_then(|st| st.next_run_ms);
    Ok(json!({
        "op": op,
        "task_name": name,
        "script_name": script_after.map(|_| script),
        "next_run": next_run,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = "schedule = \"0 3 * * *\"\nscript = \"stamp\"\n";

    #[test]
    fn an_agent_may_not_propose_fs_paths() {
        let plain = TaskDef::parse("nightly", TASK).unwrap();
        assert_eq!(paths_refused(&plain, false), None);
        let reads = TaskDef::parse("nightly", &format!("{TASK}read_paths = [\"/\"]\n")).unwrap();
        let why = paths_refused(&reads, false).unwrap();
        assert!(
            why.contains("may not propose read_paths or write_paths"),
            "{why}"
        );
        let writes =
            TaskDef::parse("nightly", &format!("{TASK}write_paths = [\"/srv/out\"]\n")).unwrap();
        let why = paths_refused(&writes, true).unwrap();
        assert!(why.contains("only the user changes"), "{why}");
        assert!(has_paths(
            "nightly",
            &format!("{TASK}write_paths = [\"/srv/out\"]\n")
        ));
        assert!(!has_paths("nightly", TASK));
    }

    #[test]
    fn a_paused_task_lists_no_next_runs() {
        let def = TaskDef::parse("nightly", TASK).unwrap();
        // 2026-01-05T00:30:00Z: the next runs are at 03:00 UTC on each day after.
        let now = 1_767_573_000_000;
        let next = next_runs(&def, now);
        assert_eq!(next.len(), 3);
        assert_eq!(next[0], 1_767_582_000_000);
        let paused = TaskDef::parse("nightly", &format!("{TASK}paused = true\n")).unwrap();
        assert!(next_runs(&paused, now).is_empty());
    }
}
