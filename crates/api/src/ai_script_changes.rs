//! Agents and the vault's saved scripts, `scripts/<name>.ws` (TV-70, S2 of TV-32).
//!
//! - `get_script { name }` reads one script: its source, why it would not compile in each way it
//!   runs, and its "Runs as" list.
//! - `propose_script_change { name, source }` and `propose_script_delete { name }` check a change
//!   and hand it to the user as a `script_proposal`, `kind` always `script`. A change must compile
//!   in every way the script runs (`Writer::script_problems`), and a delete is refused while a
//!   handler, task, device file or tool names the script, as `DELETE /api/scripts/{name}` refuses
//!   it. The proposal always waits for the user's Apply, whatever the agent's switches say: both
//!   tools are in `crate::ai_chat`'s `ALWAYS_PROPOSE`, so a trigger's run is not offered them and
//!   `Direct::covers` never covers a `script_proposal` (TV-32 Q2 O1). `POST /api/ai/apply` applies
//!   it through [`apply_script`], with `admin` or as the copy that waits in the thread.
//!
//! The "Runs as" list ([`runs_as`]) names each handler, task, device file and tool that runs the
//! script, with the hosts, secrets, `fs` paths and schedule each gives it (TV-32 Q7, R5). A change
//! to a script changes every one of them. A user that may reach any host (`allow_domains =
//! ["*"]`) is allowed, with a warning on the card (TV-32 Q4). The tool proposal card shows the
//! same list for the script its tool names (`crate::ai_scripts`).

use ai::ToolSpec;
use schema::Agent;
use serde_json::{Value, json};
use writer::{ScriptUse, Writer};

use crate::ai_chat::{ApplyError, ToolOut, str_arg, tool};

/// The tools that change a saved script. Their change always waits for the user's Apply.
pub const PROPOSE_TOOLS: [&str; 2] = ["propose_script_change", "propose_script_delete"];

/// The read and propose tools, in every agent's list like any other.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool(
            "get_script",
            "Read one of the vault's saved scripts (scripts/<name>.ws): its whole source, why it would not compile in each way it runs, and what runs it (\"runs_as\"): each type handler, task, device file and tool that names it, with the hosts, secrets, paths and schedule each gives it. Read it before proposing a change.",
            json!({ "name": { "type": "string", "description": "The script's name." } }),
        ),
        tool(
            "propose_script_change",
            "Propose a saved script (scripts/<name>.ws) for the user to apply: a new script under a new name, or the whole new source of an existing one. It must compile in every way the script runs, or it is not proposed. The change always waits for the user, who sees the diff and every handler, task, device file and tool that runs the script, and applies or discards it. A change to a script changes all of them.",
            json!({
                "name": { "type": "string", "description": "The script's name: an existing script to change, or a new name ([a-z][a-z0-9_]*) to create one." },
                "source": { "type": "string", "description": "The whole wscript source." },
            }),
        ),
        tool(
            "propose_script_delete",
            "Propose deleting a saved script for the user to apply. A script that a type handler, task, device file or tool names cannot be deleted: change or delete those first. The delete always waits for the user.",
            json!({ "name": { "type": "string", "description": "The script's name." } }),
        ),
    ]
}

/// One user of a script, as the card and the model read it.
fn use_json(u: &ScriptUse) -> Value {
    json!({
        "kind": u.kind,
        "name": u.name,
        "file": u.file,
        "event": u.event,
        "allow_domains": u.allow_domains,
        "secrets": u.secrets,
        "read_paths": u.read_paths,
        "write_paths": u.write_paths,
        "schedule": u.schedule,
        "timezone": u.timezone,
        "paused": u.paused,
    })
}

/// A user as a phrase: `task nightly`, `the before_update handler of type task`.
fn use_label(u: &ScriptUse) -> String {
    match (u.kind, &u.event) {
        ("handler", Some(e)) => format!("the {e} handler of type {}", u.name),
        ("device", Some(e)) => format!("the {e} of device file {}", u.name),
        (kind, _) => format!("{kind} {}", u.name),
    }
}

/// The warning for a run that may reach any host (TV-32 Q4, R3).
pub(crate) fn any_host_warning(what: &str) -> String {
    format!(
        "{what} may reach any host (allow_domains = [\"*\"]), including your local network and \
         this server."
    )
}

/// What runs script `name`, leaving out tool `except`, as JSON, and the warnings for it: one for
/// each user that may reach any host. One helper for every proposal card that shows a script.
pub(crate) fn runs_as(w: &Writer, name: &str, except: Option<&str>) -> (Value, Vec<String>) {
    described(&w.script_runs_as(name, except).unwrap_or_default())
}

/// The users of a script as JSON, and a warning for each that may reach any host. The task
/// proposal card uses it with the task itself left out (TV-71).
pub(crate) fn described(uses: &[ScriptUse]) -> (Value, Vec<String>) {
    let warnings = uses
        .iter()
        .filter(|u| u.allow_domains.iter().any(|d| d == "*"))
        .map(|u| {
            let mut label = use_label(u);
            if let Some(first) = label.get(..1) {
                label = format!("{}{}", first.to_uppercase(), &label[1..]);
            }
            any_host_warning(&label)
        })
        .collect();
    (uses.iter().map(use_json).collect(), warnings)
}

fn out(content: Value, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [] }),
        proposal: proposal.map(|p| ("script_proposal", p)),
    }
}

/// The error for a script that does not exist, naming those that do.
fn missing(w: &Writer, name: &str) -> ToolOut {
    let names = w.scripts().unwrap_or_default();
    ToolOut::error(if names.is_empty() {
        format!("there is no script {name:?}: the vault has no saved scripts")
    } else {
        format!(
            "there is no script {name:?}; the vault's scripts are {}",
            names.join(", ")
        )
    })
}

/// A script name, checked.
fn name_arg(input: &Value) -> Result<&str, ToolOut> {
    let name = str_arg(input, "name")?;
    if schema::is_name(name) {
        Ok(name)
    } else {
        Err(ToolOut::error(format!(
            "{name:?} is not a valid script name: use [a-z][a-z0-9_]*, at most 64 characters"
        )))
    }
}

/// `get_script`.
pub(crate) fn get(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = name_arg(input)?;
    let source = w.read_script(name).map_err(|_| missing(w, name))?;
    let problems = w
        .script_problems(name, &source)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let (users, warnings) = runs_as(w, name, None);
    Ok(out(
        json!({
            "name": name,
            "source": source,
            "problems": crate::defs::problems_json(&problems),
            "runs_as": users,
            "warnings": warnings,
        }),
        None,
    ))
}

/// `propose_script_change`: check that the source compiles in every way the script runs, and hand
/// it with its "Runs as" list to the user.
pub(crate) fn propose_change(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = name_arg(input)?;
    let after = input["source"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolOut::error("source must be the whole wscript source"))?;
    let before = w.read_script(name).ok();
    if before.as_deref() == Some(after) {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed source is the same as the current one: there is nothing to apply.",
            }),
            None,
        ));
    }
    let problems = w
        .script_problems(name, after)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    if !problems.is_empty() {
        let mut o = ToolOut::error(format!(
            "script {name:?} does not compile in every way it runs"
        ));
        o.content = json!({
            "error": format!("script {name:?} does not compile in every way it runs"),
            "problems": crate::defs::problems_json(&problems),
            "message": "The script was not proposed. Fix the problems and propose it again.",
        })
        .to_string();
        return Err(o);
    }
    let (users, warnings) = runs_as(w, name, None);
    let exists = before.is_some();
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": exists,
        "runs_as": users,
        "warnings": warnings,
        "message": "The script compiles in every way it runs and is shown to the user as a proposal, with what runs it. It has not been written: it awaits the user, who applies or discards it.",
    });
    let proposal = json!({
        "id": call_id,
        "kind": "script",
        "op": if exists { "change" } else { "create" },
        "name": name,
        "exists": exists,
        "before": before,
        "after": after,
        "runs_as": users,
        "warnings": warnings,
        "proposer": by.name,
    });
    Ok(out(content, Some(proposal)))
}

/// `propose_script_delete`: refused while anything runs the script.
pub(crate) fn propose_delete(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = name_arg(input)?;
    let before = w.read_script(name).map_err(|_| missing(w, name))?;
    let uses = w
        .script_runs_as(name, None)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    if !uses.is_empty() {
        let files: Vec<String> = uses.iter().map(use_label).collect();
        let mut o = ToolOut::error(format!(
            "script {name:?} cannot be deleted: {} runs it",
            files.join(", ")
        ));
        o.content = json!({
            "error": format!("script {name:?} cannot be deleted while something runs it"),
            "runs_as": uses.iter().map(use_json).collect::<Vec<_>>(),
            "message": "The delete was not proposed. Change or delete what runs the script first.",
        })
        .to_string();
        return Err(o);
    }
    let proposal = json!({
        "id": call_id,
        "kind": "script",
        "op": "delete",
        "name": name,
        "exists": true,
        "before": before,
        "after": null,
        "runs_as": [],
        "warnings": [],
        "proposer": by.name,
    });
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.",
        }),
        Some(proposal),
    ))
}

/// Apply a `script_proposal`. Answers `{op, script_name}`, `op` `script_create`, `script_change` or
/// `script_delete`. The source, the compile check and the "Runs as" list are checked again under
/// the writer's lock, with the write: a script changed since, a change that no longer compiles in
/// every way it runs, or a list that changed since the card is answered 409 `stale`, and nothing is
/// written. A delete is refused, 409 `script_in_use`, when something has started to run the script.
pub(crate) fn apply_script(w: &Writer, p: &Value) -> Result<Value, ApplyError> {
    let name = p["name"]
        .as_str()
        .filter(|s| schema::is_name(s))
        .ok_or_else(|| ApplyError::Invalid("the change has no script name".into()))?;
    let before = p["before"].as_str();
    let delete = p["op"] == "delete";
    let after = if delete {
        None
    } else {
        Some(
            p["after"]
                .as_str()
                .ok_or_else(|| ApplyError::Invalid("the change has no after".into()))?,
        )
    };
    let shown = &p["runs_as"];
    w.commit_checked(|w| -> Result<Vec<store::Change>, ApplyError> {
        if w.read_script(name).ok().as_deref() != before {
            return Err(ApplyError::Stale(format!(
                "the script {name} changed since the proposal, so nothing was written"
            )));
        }
        // A delete in use is refused first, as `script_in_use`.
        let changes = w.script_changes(name, after)?;
        if let Some(source) = after {
            let problems = w.script_problems(name, source)?;
            if !problems.is_empty() {
                return Err(ApplyError::Stale(format!(
                    "the script {name} does not compile in every way it runs now, so nothing was written: propose it again"
                )));
            }
        }
        if runs_as(w, name, None).0 != *shown {
            return Err(ApplyError::Stale(format!(
                "what runs the script {name} changed since the proposal, so nothing was written"
            )));
        }
        Ok(changes)
    })?;
    let op = if delete {
        "script_delete"
    } else if before.is_some() {
        "script_change"
    } else {
        "script_create"
    };
    Ok(json!({ "op": op, "script_name": name }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_reads_as_a_phrase() {
        let handler = ScriptUse {
            kind: "handler",
            name: "task".into(),
            event: Some("before_update".into()),
            ..ScriptUse::default()
        };
        assert_eq!(
            use_label(&handler),
            "the before_update handler of type task"
        );
        let device = ScriptUse {
            kind: "device",
            name: "android".into(),
            event: Some("command send_sms".into()),
            ..ScriptUse::default()
        };
        assert_eq!(
            use_label(&device),
            "the command send_sms of device file android"
        );
        let task = ScriptUse {
            kind: "task",
            name: "nightly".into(),
            ..ScriptUse::default()
        };
        assert_eq!(use_label(&task), "task nightly");
    }
}
