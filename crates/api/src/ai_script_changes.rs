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
//! - A change is refused, at the proposal and again at Apply, while any run of the script has
//!   `read_paths` or `write_paths`: the agent's code would reach those folders (TV-32 Q3, TV-87).
//!   [`shared_paths_refused`] names each such run. A delete is not affected.
//! - A change that adds a call to `device::sms_send` is refused, at the proposal and again at Apply
//!   ([`sms_send_refused`], TV-98, TV-72 policy P-a), whatever runs the script.
//! - A change to a script that a device file runs carries `device` ([`on_phone`], TV-98, TV-72
//!   U2-U4): the line [`PHONE_NOTE`], each paired device of the platform with its SMS send and
//!   contacts write switches, and what the code can do on the phone, each item marked `new` when
//!   the current code cannot do it. Apply answers 409 `stale` when it changed since the card.
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
            "Propose a saved script (scripts/<name>.ws) for the user to apply: a new script under a new name, or the whole new source of an existing one. It must compile in every way the script runs, or it is not proposed. The change always waits for the user, who sees the diff and every handler, task, device file and tool that runs the script, and applies or discards it. A change to a script changes all of them. A script that a device file names runs on the user's phone, and a change that adds a call to device::sms_send is refused.",
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

/// The refusal of a new source for script `script` when a run of it has `fs` paths (TV-71 review
/// F1, TV-87, TV-32 Q3): the agent's code would run with those folders. Task `except_task` is left
/// out: a task's proposed file has no paths, and its current one is not what runs. Only tasks have
/// `fs` paths; the refusal names each run with its folders.
pub(crate) fn shared_paths_refused(
    w: &Writer,
    script: &str,
    except_task: Option<&str>,
) -> Option<String> {
    let with_paths: Vec<String> = w
        .script_runs_as(script, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|u| !(u.kind == "task" && Some(u.name.as_str()) == except_task))
        .filter(|u| !u.read_paths.is_empty() || !u.write_paths.is_empty())
        .map(|u| {
            let mut paths = Vec::new();
            if !u.read_paths.is_empty() {
                paths.push(format!("reads {}", u.read_paths.join(", ")));
            }
            if !u.write_paths.is_empty() {
                paths.push(format!("writes {}", u.write_paths.join(", ")));
            }
            format!("{} {} ({})", u.kind, u.name, paths.join(", "))
        })
        .collect();
    if with_paths.is_empty() {
        return None;
    }
    Some(format!(
        "an agent may not change script {script:?}: {} runs it with read_paths or write_paths, so the new code would reach those folders. Tell the user the change to make in the editor",
        with_paths.join(", ")
    ))
}

/// The line a card shows for a script that a device file runs (TV-98, TV-72 U4): the phone pulls
/// the new code at its next sync and runs it with nothing more to approve there.
pub(crate) const PHONE_NOTE: &str =
    "Runs on your phone at its next sync, with no further prompt there.";

pub(crate) fn device_note(uses: &[ScriptUse]) -> &'static str {
    if uses.iter().any(|u| u.kind == "device" && u.name == "linux") {
        "Runs on your paired devices at their next sync, with no further prompt there. Linux includes the desktop app."
    } else {
        PHONE_NOTE
    }
}

/// What code that runs on a phone can do, as the card lists it (TV-98, TV-72 U3): a key, the
/// words, the `device` functions and the `vault` functions that give it.
type Capability = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

const CAPABILITIES: [Capability; 7] = [
    ("sms_send", "Send SMS", &["sms_send"], &[]),
    (
        "sms_read",
        "Read all SMS, including the codes and people that capture skips",
        &["sms_list", "sms_get"],
        &[],
    ),
    ("calls_read", "Read calls", &["calls_list"], &[]),
    (
        "contacts_read",
        "Read contacts",
        &["contacts_list", "contacts_get"],
        &[],
    ),
    (
        "contacts_write",
        "Add or delete contacts",
        &["contacts_upsert", "contacts_delete"],
        &[],
    ),
    (
        "notifications",
        "Show phone notifications",
        &["show_notification"],
        &[],
    ),
    (
        "notes_write",
        "Write notes",
        &[],
        &[
            "create",
            "update",
            "patch",
            "append_text",
            "append_blocks",
            "delete",
            "restore",
            "archive",
            "unarchive",
            "upload_asset",
            "add_term",
            "daily",
        ],
    ),
];

/// The words of wscript source: identifiers, `::`, and each other character that is not white
/// space. Nothing is left out, strings and comments included: a string's `{expr}` holes are code.
fn words(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut chars = src.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut end = i + c.len_utf8();
        if c.is_ascii_alphabetic() || c == '_' {
            while let Some(&(j, d)) = chars.peek() {
                if !(d.is_ascii_alphanumeric() || d == '_') {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
        } else if c == ':' && chars.peek().is_some_and(|&(_, d)| d == ':') {
            chars.next();
            end += 1;
        }
        out.push(&src[i..end]);
    }
    out
}

/// `src` with its `//` and `/* */` comments each replaced by `;`, so that no words meet across a
/// comment. A quote opens a string that ends at the next unescaped quote. A misread can only drop
/// code, never add it: the current source is read this way so that it never seems to do more than
/// it does.
fn without_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push_str(";\n");
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() {
                    if last == '*' && n == '/' {
                        break;
                    }
                    last = n;
                }
                out.push(';');
            }
            _ => out.push(c),
        }
    }
    out
}

/// How many times the words call or name `module::f` (or `module.f`), `use module::f` included.
fn qualified(words: &[&str], module: &str, f: &str) -> usize {
    words
        .windows(3)
        .filter(|w| w[0] == module && (w[1] == "::" || w[1] == ".") && w[2] == f)
        .count()
}

/// What `source` can do on a phone, by the keys of [`CAPABILITIES`]. `proposed` reads it at the
/// most: a `device` function's bare name anywhere counts, in a comment or a string too. The current
/// source is read at the least: only `device::f` and `vault::f` outside comments.
fn can(source: &str, proposed: bool) -> Vec<&'static str> {
    let text;
    let words = if proposed {
        words(source)
    } else {
        text = without_comments(source);
        words(&text)
    };
    CAPABILITIES
        .iter()
        .filter(|(_, _, device, vault)| {
            device.iter().any(|f| {
                if proposed {
                    words.contains(f)
                } else {
                    qualified(&words, "device", f) > 0
                }
            }) || vault.iter().any(|f| qualified(&words, "vault", f) > 0)
        })
        .map(|(key, ..)| *key)
        .collect()
}

/// Refuse new phone command sites with the same conservative count rule as SMS sends (TV-108).
pub(crate) fn device_command_refused(
    name: &str,
    before: Option<&str>,
    after: &str,
) -> Option<String> {
    let now = before.map_or(0, |b| {
        qualified(&words(&without_comments(b)), "vault", "device_command")
    });
    let then = words(after)
        .iter()
        .filter(|w| **w == "device_command")
        .count();
    if then <= now {
        return None;
    }
    Some(format!(
        "an agent may not add a call to vault::device_command to script {name:?}: the current script calls it {now} times and the proposed source names device_command {then} times. Every mention counts, in comments and strings too. Use draft_sms to draft an SMS through the Outbox for review instead"
    ))
}

/// Refuse added SMS send sites. Every proposed mention counts, but current comments do not.
pub(crate) fn sms_send_refused(name: &str, before: Option<&str>, after: &str) -> Option<String> {
    let now = before.map_or(0, |b| {
        qualified(&words(&without_comments(b)), "device", "sms_send")
    });
    let then = words(after).iter().filter(|w| **w == "sms_send").count();
    if then <= now {
        return None;
    }
    Some(format!(
        "an agent may not add a call to device::sms_send to script {name:?}: the current script calls it {now} times and the proposed source names sms_send {then} times. Code on a phone with SMS send on can text any number with no further prompt there, so a new call site is refused. Every mention of sms_send counts, in comments and strings too. Tell the user the change to make in the editor"
    ))
}

/// Where and what the new source of script `name` can do on a phone, when a device file runs it
/// (TV-98, TV-72 U2-U4), or `None`: the line for the card, the device platforms, each paired
/// device of them by name with its SMS send and contacts write switches, and what the code can
/// do, each item marked `new` when the current source cannot do it.
pub(crate) fn on_phone(
    w: &Writer,
    uses: &[ScriptUse],
    before: Option<&str>,
    after: &str,
) -> Result<Option<Value>, writer::Error> {
    let mut platforms: Vec<&str> = uses
        .iter()
        .filter(|u| u.kind == "device")
        .map(|u| u.name.as_str())
        .collect();
    platforms.sort_unstable();
    platforms.dedup();
    if platforms.is_empty() {
        return Ok(None);
    }
    let mut paired: Vec<state::Device> = w
        .state()
        .list_devices()?
        .into_iter()
        .filter(|d| platforms.contains(&d.platform.as_str()))
        .collect();
    paired.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    let devices: Vec<Value> = paired
        .into_iter()
        .map(|d| {
            let settings: Value = serde_json::from_str(&d.settings).unwrap_or(Value::Null);
            json!({
                "id": d.id,
                "name": d.name,
                "platform": d.platform,
                "sms_send": settings["sms_send"] == true,
                "contacts_write": settings["contacts_write"] == true,
            })
        })
        .collect();
    let now = before.map(|b| can(b, false)).unwrap_or_default();
    let can: Vec<Value> = can(after, true)
        .into_iter()
        .map(|key| {
            let label = CAPABILITIES.iter().find(|c| c.0 == key).map_or("", |c| c.1);
            json!({ "key": key, "label": label, "new": !now.contains(&key) })
        })
        .collect();
    Ok(Some(json!({
        "note": if sensitive_phone_source(before, Some(after)) {
            format!("{} {PHONE_ADMIN_MESSAGE}", device_note(uses))
        } else {
            device_note(uses).to_owned()
        },
        "platforms": platforms,
        "devices": devices,
        "can": can,
    })))
}

pub(crate) const PHONE_ADMIN_MESSAGE: &str = "A signed-in admin must apply this change because the current or proposed phone code contains SMS send or contact writes. API tokens, including admin-scoped tokens, cannot apply it.";

// Count bare names too, including imports, aliases, interpolation and comments. False positives
// need an admin, rather than letting a different spelling hide a sensitive call.
pub(crate) fn sensitive_phone_source(before: Option<&str>, after: Option<&str>) -> bool {
    [before, after].into_iter().flatten().any(|source| {
        words(source)
            .iter()
            .any(|word| matches!(*word, "sms_send" | "contacts_upsert" | "contacts_delete"))
    })
}

/// Call under `commit_checked` for every proposal path that writes a script. Read the actual
/// current script and device references, not the untrusted proposal's warning or before fields.
pub(crate) fn require_phone_script_admin(
    w: &Writer,
    name: &str,
    after: Option<&str>,
    admin: bool,
) -> Result<(), ApplyError> {
    if !admin
        && sensitive_phone_source(w.read_script(name).ok().as_deref(), after)
        && w.script_runs_as(name, None)?
            .iter()
            .any(|u| u.kind == "device")
    {
        return Err(ApplyError::Refused(PHONE_ADMIN_MESSAGE.into()));
    }
    Ok(())
}

/// A refusal of a proposal, with the reason as the model reads it.
fn refused(why: String) -> ToolOut {
    let mut o = ToolOut::error(why.clone());
    o.content = json!({ "error": why, "message": "The script was not proposed." }).to_string();
    o
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
    if let Some(why) = shared_paths_refused(w, name, None) {
        return Err(refused(why));
    }
    if let Some(why) = sms_send_refused(name, before.as_deref(), after) {
        return Err(refused(why));
    }
    if let Some(why) = device_command_refused(name, before.as_deref(), after) {
        return Err(refused(why));
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
    let uses = w
        .script_runs_as(name, None)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let (users, warnings) = described(&uses);
    let phone =
        on_phone(w, &uses, before.as_deref(), after).map_err(|e| ToolOut::error(e.to_string()))?;
    let exists = before.is_some();
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": exists,
        "runs_as": users,
        "warnings": warnings,
        "device": phone,
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
        "device": phone,
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
/// A change is refused, 403 `not_permitted`, while a task has `read_paths` or `write_paths` and runs
/// it (TV-87, as the proposal is).
pub(crate) fn apply_script(w: &Writer, p: &Value, admin: bool) -> Result<Value, ApplyError> {
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
        require_phone_script_admin(w, name, after, admin)?;
        if w.read_script(name).ok().as_deref() != before {
            return Err(ApplyError::Stale(format!(
                "the script {name} changed since the proposal, so nothing was written"
            )));
        }
        // A change is refused while a run of the script has folders (TV-87). A delete is not.
        if after.is_some()
            && let Some(why) = shared_paths_refused(w, name, None)
        {
            return Err(ApplyError::Refused(why));
        }
        // A change that adds a call to device::sms_send is refused (TV-98, policy P-a).
        if let Some(source) = after
            && let Some(why) = sms_send_refused(name, before, source)
        {
            return Err(ApplyError::Refused(why));
        }
        if let Some(source) = after
            && let Some(why) = device_command_refused(name, before, source)
        {
            return Err(ApplyError::Refused(why));
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
        // The phones it runs on, their switches and what the code can do, as the card showed them.
        if let Some(source) = after {
            let uses = w.script_runs_as(name, None)?;
            let phone = on_phone(w, &uses, before, source)?;
            if phone.as_ref().unwrap_or(&Value::Null) != &p["device"] {
                return Err(ApplyError::Stale(format!(
                    "the phones that run the script {name}, or their switches, changed since the proposal, so nothing was written"
                )));
            }
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

    const SENDS: &str = "use device\n\nfn send(to: string) -> Result[Value, string] {\n    device::sms_send(to, \"hi\")\n}\n";

    #[test]
    fn a_new_sms_send_call_site_is_refused_and_a_kept_one_is_not() {
        assert!(sms_send_refused("m", Some(SENDS), SENDS).is_none());
        let moved = SENDS.replace("\"hi\"", "\"hello\"");
        assert!(sms_send_refused("m", Some(SENDS), &moved).is_none());
        let two = format!(
            "{SENDS}\nfn again() -> Result[Value, string] {{\n    device::sms_send(\"1\", \"x\")\n}}\n"
        );
        let why = sms_send_refused("m", Some(SENDS), &two).unwrap();
        assert!(why.contains("calls it 1 times"), "{why}");
        assert!(why.contains("names sms_send 2 times"), "{why}");
        assert!(sms_send_refused("m", None, SENDS).is_some());
        assert!(sms_send_refused("m", Some("fn run() {}\n"), SENDS).is_some());
    }

    // TV-72 R9: no form of the call escapes the count.
    #[test]
    fn every_form_of_an_sms_send_call_is_counted() {
        let quiet = "use device\n\nfn run() -> Result[(), string] {\n    Ok(())\n}\n";
        for added in [
            "let f = device::sms_send\n",
            "use device::sms_send\n",
            "device :: sms_send(\"1\", \"x\")\n",
            "device /* x */ :: sms_send(\"1\", \"x\")\n",
            "let s = \"{device::sms_send(\\\"1\\\", \\\"x\\\")}\"\n",
            "let u = \"http://x\" device::sms_send(\"1\", \"x\")\n",
        ] {
            let after = format!("{quiet}{added}");
            assert!(
                sms_send_refused("m", Some(quiet), &after).is_some(),
                "{added}"
            );
        }
    }

    // A mention of sms_send in a comment of the current script is not a call: replacing it with a
    // call is refused.
    #[test]
    fn a_commented_call_in_the_current_script_does_not_count() {
        let before =
            "use device\n// device::sms_send(to, body) once lived here\n/* device::sms_send */\n";
        let after = "use device\nfn f() -> Result[Value, string] {\n    device::sms_send(\"1\", \"x\")\n}\n";
        assert!(sms_send_refused("m", Some(before), after).is_some());
        let url = "let u = \"http://x\"\ndevice::sms_send(\"1\", \"x\")\n";
        assert_eq!(
            qualified(&words(&without_comments(url)), "device", "sms_send"),
            1
        );
    }

    #[test]
    fn what_code_can_do_on_a_phone() {
        let src = "use vault\nuse device\n\nfn f() {\n    device::sms_list(x)\n    device::contacts_delete(\"1\")\n    vault::create(\"note\", x)\n}\n";
        assert_eq!(
            can(src, true),
            ["sms_read", "contacts_write", "notes_write"]
        );
        assert_eq!(
            can(src, false),
            ["sms_read", "contacts_write", "notes_write"]
        );
        // The current source is read at the least: a comment does not count.
        assert!(can("// device::show_notification(a, b, c)\n", false).is_empty());
        assert_eq!(
            can("// device::show_notification(a, b, c)\n", true),
            ["notifications"]
        );
        assert_eq!(can("use device::calls_list\n", false), ["calls_read"]);
    }
}
