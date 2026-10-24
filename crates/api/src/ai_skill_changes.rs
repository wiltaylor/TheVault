//! Agents that manage skills (skills S9): `list_skills`, `get_skill`, `propose_skill_change`,
//! `propose_skill_file_change`, `propose_skill_file_delete` and `propose_skill_delete`, and the
//! apply of a `skill_proposal`.
//!
//! A proposal is checked with the writer's dry runs ([`Writer::check_skill_md`] and
//! [`Writer::check_skill_file`]), so it meets the refusals the `/api/skills` writes give: the
//! `SKILL.md` constraints (skills S2), the name, the path rules and the limits (skills S1). Each
//! refusal goes back to the model as an `is_error` result with the issues. The proposal records
//! `base`, a fingerprint of what it replaces (the `SKILL.md`, the file, the files under a folder,
//! or the whole skill for a delete; `null` for nothing), so `POST /api/ai/apply` re-reads first
//! and answers 409 `stale` when it changed since. Applied, a change goes through the writer calls
//! the `/api/skills` routes use: [`Writer::put_skill_md`], [`Writer::put_skill_file`],
//! [`Writer::delete_skill_file`] and [`Writer::delete_skill`], which record `skill.change`.
//!
//! The `skill_proposal` event is `{id, kind: "skill", op, name, path?, exists, before, after,
//! issues, base, ...}`, `op` one of `create`, `change`, `file`, `file_delete` and `delete`:
//!
//! - `create` and `change`: `before` the `SKILL.md` text or `null`, `after` the new text, `issues`
//!   its warnings, `description` the new frontmatter description;
//! - `file`: `path`, `binary`, `size` and `media_type` of the new file, `before` and `after` its
//!   text when it is text (`before` `null` for a new or binary file), `before_size` and
//!   `before_media_type` of the file it replaces (`null` for a new one), and for a binary file
//!   `base64`, the bytes to write;
//! - `file_delete`: `path`, `files` the paths it deletes, `before` the file's text when it is one
//!   text file;
//! - `delete`: `before` the `SKILL.md` text, `files` and `size`.
//!
//! The apply answers `{op, skill_name, path?}`, `op` one of `skill_create`, `skill_change`,
//! `skill_file`, `skill_file_delete` and `skill_delete`. After a create the agent may call
//! `use_skill` on the skill at once when its `permissions.skills` offers it; the thread's skills
//! snapshot is not changed (the user's `/update-skills` refreshes it).

use ai::ToolSpec;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use schema::Agent;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use writer::Writer;

use crate::ai_chat::{ApplyError, ToolOut, str_arg, tool};
use crate::error::issue_json;
use crate::skills::{skill_json, summary_json};

/// The tools that change a skill, under the `skills` confirmation switch.
pub const PROPOSE_TOOLS: [&str; 4] = [
    "propose_skill_change",
    "propose_skill_file_change",
    "propose_skill_file_delete",
    "propose_skill_delete",
];

/// The longest file text a proposal shows the user as a diff; a longer text file is shown by its
/// size, like a binary one.
const DIFF_LIMIT: u64 = 256 * 1024;

/// What the model reads before writing a `SKILL.md`.
const FORMAT: &str = "A skill is a folder skills/<name>/ holding SKILL.md and any other files, conventionally scripts/, references/ and assets/ (the Agent Skills specification, https://agentskills.io/specification). SKILL.md is YAML frontmatter between two `---` lines, then Markdown instructions. Frontmatter: `name` (required, 1 to 64 characters, lowercase a-z, 0-9 and hyphens, not starting or ending with a hyphen, no `--`, equal to the folder name), `description` (required, 1 to 1024 characters: what the skill does and when to use it), `license` (optional), `compatibility` (optional, 1 to 500 characters), `metadata` (optional, a map of string keys to string values) and `allowed-tools` (optional, a space-separated string, recorded but granting nothing). The body is what use_skill answers: write instructions an agent can follow, pointing to the skill's other files by their paths from the folder. Limits: 200 files and 20 MiB a skill, 2 MiB a text file read into a model, no symlinks.";

/// The model's tools for managing skills.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        tool(
            "list_skills",
            "List every skill in the vault, offered to you or not: its name, description, license, compatibility, metadata, allowed-tools, file count, size, whether it is valid and its issues, and whether you are offered it (use_skill opens only those).",
            json!({}),
        ),
        tool(
            "get_skill",
            "Read one skill: its SKILL.md text as stored, the frontmatter fields, the body, its file list with sizes and media types, its issues, and the skill format. Read it before proposing a change; read a file's text with read_skill_file when you are offered the skill.",
            json!({ "name": { "type": "string", "description": "The skill's name, such as `pdf-forms`." } }),
        ),
        tool(
            "propose_skill_change",
            "Propose a skill's SKILL.md for the user to apply: a new skill under a new name, or the whole new SKILL.md of an existing one. It is checked as the skill write would check it (the frontmatter's name must equal `name`; a description is required); an invalid one is refused with its issues. The user sees a diff and applies or discards it. When the user lets skill changes go ahead without asking, it is written at once and the result says applied. Add the skill's other files with propose_skill_file_change.",
            json!({
                "name": { "type": "string", "description": "The skill's name: an existing skill to change, or a new name (lowercase letters, digits and hyphens) to create one." },
                "skill_md": { "type": "string", "description": "The whole SKILL.md text: `---`, the YAML frontmatter, `---`, then the Markdown instructions." },
            }),
        ),
        tool(
            "propose_skill_file_change",
            "Propose a file of a skill for the user to apply: a new file, or the whole new content of one, at a path from the skill's folder such as `scripts/fill.py` or `references/forms.md`. Give exactly one of `text` (the file as text) or `base64` (its bytes, base64-encoded, for a binary file). SKILL.md is refused: change it with propose_skill_change. The skill must exist when the change is applied, so for a new skill the user applies its SKILL.md first. When the user lets skill changes go ahead without asking, it is written at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The skill's name." },
                "path": { "type": "string", "description": "The file's path from the skill's folder." },
                "text": { "type": ["string", "null"], "description": "The whole file as text; null when giving base64." },
                "base64": { "type": ["string", "null"], "description": "The whole file's bytes, base64-encoded; null when giving text." },
            }),
        ),
        tool(
            "propose_skill_file_delete",
            "Propose deleting a file of a skill, or every file under a folder of it, for the user to apply. SKILL.md cannot be deleted on its own: propose deleting the skill. When the user lets skill changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({
                "name": { "type": "string", "description": "The skill's name." },
                "path": { "type": "string", "description": "The file's or folder's path from the skill's folder." },
            }),
        ),
        tool(
            "propose_skill_delete",
            "Propose deleting a whole skill, its folder and every file in it, for the user to apply. When the user lets skill changes go ahead without asking, it is deleted at once and the result says applied.",
            json!({ "name": { "type": "string", "description": "The skill's name." } }),
        ),
    ]
}

/// A skill tool's answer: `content` for the model, and the skill names for the summary.
fn out(content: Value, names: Vec<Value>, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [], "skills": names }),
        proposal: proposal.map(|p| ("skill_proposal", p)),
    }
}

/// A refusal from the writer as an `is_error` result: its message, the issues the API would
/// answer, and what the model should do.
fn refused(e: writer::Error, then: &str) -> ToolOut {
    let message = e.to_string();
    let details = crate::ApiError::from(e).body;
    let mut out = ToolOut::error(message.clone());
    out.content = json!({ "error": message, "issues": details, "message": then }).to_string();
    out
}

fn missing(e: writer::Error) -> ToolOut {
    match e {
        writer::Error::Missing(m) => ToolOut::error(format!("{m}: list_skills lists them")),
        e => ToolOut::error(e.to_string()),
    }
}

/// Whether `agent` is offered skill `name` (skills S6), for its `use_skill`.
fn offered_to(agent: &Agent, name: &str) -> bool {
    crate::ai_skills::uses_skills(agent) && agent.permissions.allows_skill(name)
}

/// `list_skills`.
pub(crate) fn list(w: &Writer, agent: &Agent) -> Result<ToolOut, ToolOut> {
    let skills = w.skills().map_err(|e| ToolOut::error(e.to_string()))?;
    let rows: Vec<Value> = skills
        .iter()
        .map(|s| {
            let mut row = summary_json(s);
            row["offered"] = json!(s.valid() && offered_to(agent, &s.name));
            row
        })
        .collect();
    let names = skills.iter().map(|s| json!(s.name)).collect();
    Ok(out(json!({ "skills": rows }), names, None))
}

/// `get_skill`.
pub(crate) fn get(w: &Writer, agent: &Agent, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let skill = w.skill(name).map_err(missing)?;
    let mut content = skill_json(&skill);
    content["skill_md"] = json!(skill_md(w, name));
    content["offered"] = json!(skill.valid() && offered_to(agent, name));
    content["format"] = json!(FORMAT);
    Ok(out(content, vec![json!(name)], None))
}

/// The `SKILL.md` text of skill `name`, if it has one that reads as text.
fn skill_md(w: &Writer, name: &str) -> Option<String> {
    w.skill_text(name, store::SKILL_FILE).ok()
}

/// A fingerprint of skill `name`'s files at `path` (the file, or every file under the folder),
/// or of all its files when `path` is `None`; `None` when there are none.
fn fingerprint(
    w: &Writer,
    name: &str,
    path: Option<&str>,
) -> Result<Option<String>, writer::Error> {
    let skill = match w.skill(name) {
        Ok(s) => s,
        Err(writer::Error::Missing(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut h = Sha256::new();
    let mut any = false;
    for f in &skill.files {
        let keep = path.is_none_or(|p| f.path == p || f.path.starts_with(&format!("{p}/")));
        if !keep {
            continue;
        }
        any = true;
        let bytes = w.skill_file(name, &f.path)?;
        h.update(f.path.as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(&bytes);
    }
    Ok(any.then(|| URL_SAFE_NO_PAD.encode(h.finalize())))
}

fn fingerprint_or_error(
    w: &Writer,
    name: &str,
    path: Option<&str>,
) -> Result<Option<String>, ToolOut> {
    fingerprint(w, name, path).map_err(|e| ToolOut::error(e.to_string()))
}

/// What the tool result says about `use_skill` once the skill exists.
fn use_note(agent: &Agent, name: &str) -> String {
    if offered_to(agent, name) {
        format!(
            "Once it exists you may call use_skill on {name} at once, though the Available skills list at the end of your instructions stays as it is until the user types /update-skills."
        )
    } else {
        format!(
            "Your permissions do not offer the skill {name}, so use_skill will not open it; another agent offered it can use it."
        )
    }
}

/// `propose_skill_change`: check the `SKILL.md` as the skill write would, without writing it,
/// and hand it to the user.
pub(crate) fn propose_change(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let text = input["skill_md"]
        .as_str()
        .ok_or_else(|| ToolOut::error("skill_md must be the whole SKILL.md text"))?;
    let (warnings, created) = w.check_skill_md(name, text).map_err(|e| {
        refused(
            e,
            "The skill was not proposed. Fix the issues and propose it again; get_skill answers the skill format.",
        )
    })?;
    let before = if created { None } else { skill_md(w, name) };
    let names = vec![json!(name)];
    if before.as_deref() == Some(text) {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed SKILL.md is the same as the current one: there is nothing to apply.",
            }),
            names,
            None,
        ));
    }
    let base = fingerprint_or_error(w, name, Some(store::SKILL_FILE))?;
    let description = schema::parse_skill_md(text, name).0.description;
    let issues: Vec<Value> = warnings.iter().map(issue_json).collect();
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": !created,
        "warnings": issues,
        "message": format!(
            "{} {}",
            if created {
                "The new skill is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who applies or discards it."
            } else {
                "The SKILL.md change is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it."
            },
            use_note(by, name)
        ),
    });
    let proposal = json!({
        "id": call_id,
        "kind": "skill",
        "op": if created { "create" } else { "change" },
        "name": name,
        "path": store::SKILL_FILE,
        "exists": !created,
        "description": description,
        "before": before,
        "after": text,
        "issues": issues,
        "base": base,
        "proposer": by.name,
    });
    Ok(out(content, names, Some(proposal)))
}

/// Whether `bytes` read as text: UTF-8 with no NUL.
fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// `propose_skill_file_change`: check the file as the skill file write would, without writing
/// it, and hand it to the user.
pub(crate) fn propose_file(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let raw = str_arg(input, "path")?;
    let path = store::normalise_skill_path(raw).map_err(|e| {
        ToolOut::error(format!(
            "{e}: give a path inside the skill's folder, such as scripts/run.py"
        ))
    })?;
    if path == store::SKILL_FILE {
        return Err(ToolOut::error(
            "SKILL.md is the skill's instructions: propose it with propose_skill_change, as the whole SKILL.md text.",
        ));
    }
    let bytes = match (input["text"].as_str(), input["base64"].as_str()) {
        (Some(t), None) => t.as_bytes().to_vec(),
        (None, Some(b)) => {
            let clean: String = b.chars().filter(|c| !c.is_whitespace()).collect();
            STANDARD
                .decode(clean.as_bytes())
                .map_err(|e| ToolOut::error(format!("base64 is not valid base64: {e}")))?
        }
        _ => {
            return Err(ToolOut::error(
                "give exactly one of text or base64, the other null",
            ));
        }
    };
    let exists = w
        .check_skill_file(name, &path, bytes.len() as u64)
        .map_err(|e| {
            refused(
                e,
                "The file was not proposed. Fix the problem and propose it again.",
            )
        })?;
    let names = vec![json!(name)];
    let current = if exists {
        w.skill(name).ok().and_then(|s| s.file(&path).cloned())
    } else {
        None
    };
    let current_bytes = match &current {
        Some(_) => Some(
            w.skill_file(name, &path)
                .map_err(|e| ToolOut::error(e.to_string()))?,
        ),
        None => None,
    };
    if current_bytes.as_deref() == Some(bytes.as_slice()) {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed file is the same as the current one: there is nothing to apply.",
            }),
            names,
            None,
        ));
    }
    let shown = |b: &[u8]| {
        (b.len() as u64 <= DIFF_LIMIT && is_text(b))
            .then(|| String::from_utf8_lossy(b).into_owned())
    };
    let after_text = shown(&bytes);
    let binary = !is_text(&bytes);
    let base = fingerprint_or_error(w, name, Some(&path))?;
    let media_type = writer::skill_media_type(&path);
    let mut proposal = json!({
        "id": call_id,
        "kind": "skill",
        "op": "file",
        "name": name,
        "path": path,
        "exists": current.is_some(),
        "skill_exists": exists,
        "binary": binary,
        "size": bytes.len(),
        "media_type": media_type,
        "before": current_bytes.as_deref().and_then(shown),
        "after": after_text,
        "before_size": current.as_ref().map(|f| f.size),
        "before_media_type": current.as_ref().map(|f| f.media_type),
        "issues": [],
        "base": base,
        "proposer": by.name,
    });
    if after_text.is_none() {
        // Too long to show, or binary: the bytes travel as base64.
        proposal["base64"] = json!(STANDARD.encode(&bytes));
    }
    let then = if exists {
        String::new()
    } else {
        format!(
            " The skill {name} does not exist yet: the user must apply its SKILL.md (propose_skill_change) before this file."
        )
    };
    let content = json!({
        "proposed": true,
        "name": name,
        "path": path,
        "size": bytes.len(),
        "exists": current.is_some(),
        "message": format!(
            "The file is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it.{then}"
        ),
    });
    Ok(out(content, names, Some(proposal)))
}

/// `propose_skill_file_delete`.
pub(crate) fn propose_file_delete(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let raw = str_arg(input, "path")?;
    let path = store::normalise_skill_path(raw)
        .map_err(|e| ToolOut::error(format!("{e}: give a path inside the skill's folder")))?;
    if path == store::SKILL_FILE {
        return Err(ToolOut::error(
            "SKILL.md makes the folder a skill and cannot be deleted on its own: propose_skill_delete deletes the skill.",
        ));
    }
    let skill = w.skill(name).map_err(missing)?;
    let prefix = format!("{path}/");
    let files: Vec<&writer::SkillFileInfo> = skill
        .files
        .iter()
        .filter(|f| f.path == path || f.path.starts_with(&prefix))
        .collect();
    if files.is_empty() {
        return Err(ToolOut::error(format!(
            "skill {name:?} has no file {path:?}: get_skill lists its files"
        )));
    }
    if files.iter().any(|f| f.path == store::SKILL_FILE) {
        return Err(ToolOut::error(
            "SKILL.md makes the folder a skill and cannot be deleted on its own: propose_skill_delete deletes the skill.",
        ));
    }
    let before = match files.as_slice() {
        [f] if f.path == path && f.text && f.size <= DIFF_LIMIT => w.skill_text(name, &path).ok(),
        _ => None,
    };
    let base = fingerprint_or_error(w, name, Some(&path))?;
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let size: u64 = files.iter().map(|f| f.size).sum();
    let proposal = json!({
        "id": call_id,
        "kind": "skill",
        "op": "file_delete",
        "name": name,
        "path": path,
        "exists": true,
        "files": paths,
        "size": size,
        "media_type": (files.len() == 1).then(|| files[0].media_type),
        "before": before,
        "after": null,
        "issues": [],
        "base": base,
        "proposer": by.name,
    });
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "files": paths,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.",
        }),
        vec![json!(name)],
        Some(proposal),
    ))
}

/// `propose_skill_delete`.
pub(crate) fn propose_delete(
    w: &Writer,
    by: &Agent,
    call_id: &str,
    input: &Value,
) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let skill = w.skill(name).map_err(missing)?;
    let base = fingerprint_or_error(w, name, None)?;
    let paths: Vec<&str> = skill.files.iter().map(|f| f.path.as_str()).collect();
    let proposal = json!({
        "id": call_id,
        "kind": "skill",
        "op": "delete",
        "name": name,
        "exists": true,
        "description": skill.md.description,
        "files": paths,
        "size": skill.size,
        "before": skill_md(w, name),
        "after": null,
        "issues": [],
        "base": base,
        "proposer": by.name,
    });
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "files": paths.len(),
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.",
        }),
        vec![json!(name)],
        Some(proposal),
    ))
}

/// Apply a `skill_proposal` (skills S9), when what it replaces is still as it was proposed.
pub(crate) fn apply_skill(w: &Writer, p: &Value) -> Result<Value, ApplyError> {
    let member = |k: &str| {
        p[k].as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApplyError::Invalid(format!("the change has no {k}")))
    };
    let name = member("name")?;
    let op = member("op")?;
    let path = match op {
        "create" | "change" => Some(store::SKILL_FILE.to_owned()),
        "file" | "file_delete" => Some(
            store::normalise_skill_path(member("path")?)
                .map_err(|e| ApplyError::Invalid(e.to_string()))?,
        ),
        "delete" => None,
        other => {
            return Err(ApplyError::Invalid(format!(
                "{other:?} is not a skill change"
            )));
        }
    };
    if fingerprint(w, name, path.as_deref())?.as_deref() != p["base"].as_str() {
        return Err(ApplyError::Stale(format!(
            "the skill {name} changed since the proposal, so nothing was written"
        )));
    }
    match op {
        "create" | "change" => {
            let (_, created) = w.put_skill_md(name, member("after")?)?;
            let op = if created {
                "skill_create"
            } else {
                "skill_change"
            };
            Ok(json!({ "op": op, "skill_name": name }))
        }
        "file" => {
            let path = path.unwrap_or_default();
            let bytes = match (p["base64"].as_str(), p["after"].as_str()) {
                (Some(b), _) => STANDARD
                    .decode(b)
                    .map_err(|e| ApplyError::Invalid(format!("the file's base64 is bad: {e}")))?,
                (None, Some(t)) => t.as_bytes().to_vec(),
                (None, None) => return Err(ApplyError::Invalid("the change has no file".into())),
            };
            if matches!(w.skill(name), Err(writer::Error::Missing(_))) {
                return Err(ApplyError::Write(writer::Error::Missing(format!(
                    "there is no skill {name:?} yet: apply its SKILL.md first"
                ))));
            }
            let file = w.put_skill_file(name, &path, &bytes)?;
            Ok(json!({ "op": "skill_file", "skill_name": name, "path": file.path }))
        }
        "file_delete" => {
            let path = path.unwrap_or_default();
            let deleted = w.delete_skill_file(name, &path)?;
            Ok(json!({
                "op": "skill_file_delete", "skill_name": name, "path": path, "deleted": deleted,
            }))
        }
        _ => {
            w.delete_skill(name)?;
            Ok(json!({ "op": "skill_delete", "skill_name": name }))
        }
    }
}

/// What the model is told of a skill change made without asking (ai D22).
pub(crate) fn made_line(done: &Value) -> String {
    let name = done["skill_name"].as_str().unwrap_or_default();
    let path = done["path"].as_str().unwrap_or_default();
    let after = "You may call use_skill on it now when your permissions offer it; the Available skills list at the end of your instructions stays as it is until the user types /update-skills.";
    match done["op"].as_str().unwrap_or_default() {
        "skill_create" => format!("The skill {name} was created. {after}"),
        "skill_change" => format!("The SKILL.md of skill {name} was changed. {after}"),
        "skill_file" => format!("The file {path} of skill {name} was written."),
        "skill_file_delete" => format!("{path} was deleted from skill {name}."),
        _ => format!("The skill {name} was deleted."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_are_known() {
        let names: Vec<String> = tool_specs().into_iter().map(|t| t.name).collect();
        assert_eq!(names.len(), 6);
        assert!(PROPOSE_TOOLS.iter().all(|n| names.iter().any(|m| m == n)));
    }

    #[test]
    fn text_is_told_from_binary() {
        assert!(is_text(b"hello"));
        assert!(!is_text(&[0, 1, 2]));
        assert!(!is_text(&[0xff, 0xfe]));
    }
}
