//! The chat's taxonomy tools (ai D31): `list_taxonomies`, `get_taxonomy`, `taxonomy_format`,
//! `propose_taxonomy_change` and `propose_taxonomy_delete`, and the apply of a
//! `taxonomy_proposal`.
//!
//! A proposal is checked with the writer's dry runs ([`Writer::check_taxonomy`] and
//! [`Writer::check_delete_taxonomy`]), so it meets the refusals `PUT` and `DELETE
//! /api/taxonomies/{name}` give: invalid WCL, a rename of a term that is not there, a dropped term
//! a note holds, and a delete of a taxonomy a type uses. Each goes back to the model as an
//! `is_error` result. Applied, a change goes through [`Writer::put_taxonomy`] with its renames and
//! a delete through [`Writer::delete_taxonomy`], after checking the file is still the `before`
//! the proposal showed.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{Value, json};
use state::Author;
use writer::{BUILT_IN_TAXONOMIES, Writer};

use crate::ai_chat::{ApplyError, ToolOut, VAULT_FORMAT, str_arg};

/// A taxonomy tool's answer: `content` for the model, and the taxonomy names for the summary.
fn out(content: Value, names: Vec<Value>, proposal: Option<Value>) -> ToolOut {
    ToolOut {
        content: content.to_string(),
        is_error: false,
        summary: json!({ "notes": [], "taxonomies": names }),
        proposal: proposal.map(|p| ("taxonomy_proposal", p)),
    }
}

/// A refusal from the writer as an `is_error` result: its message, the details the API would
/// answer (the notes a refusal names among them), and what the model should do.
fn refused(w: &Writer, e: writer::Error, then: &str) -> ToolOut {
    let message = e.to_string();
    let details = crate::error::in_use(w, e).body;
    let mut out = ToolOut::error(message.clone());
    out.content = json!({ "error": message, "issues": details, "message": then }).to_string();
    out
}

fn is_built_in(name: &str) -> bool {
    BUILT_IN_TAXONOMIES.iter().any(|(n, _)| *n == name)
}

/// `list_taxonomies`: each taxonomy's name, label, description, whether it is open, whether it
/// is built in, and its term count.
pub(crate) fn list(w: &Writer) -> ToolOut {
    let schema = w.schema();
    let mut all: Vec<&schema::Taxonomy> = schema.taxonomies().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    let rows: Vec<Value> = all
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "label": t.label,
                "description": t.description,
                "open": t.allow_new,
                "builtin": is_built_in(&t.name),
                "terms": t.paths().len(),
            })
        })
        .collect();
    let names = all.iter().map(|t| json!(t.name)).collect();
    out(json!({ "taxonomies": rows }), names, None)
}

/// `get_taxonomy`: the file as stored and every term as a path.
pub(crate) fn get(w: &Writer, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = w
        .read_taxonomy(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    let schema = w.schema();
    let t = schema.taxonomy(name);
    Ok(out(
        json!({
            "name": name,
            "wcl": wcl,
            "label": t.map(|t| t.label.clone()),
            "description": t.and_then(|t| t.description.clone()),
            "open": t.is_some_and(|t| t.allow_new),
            "builtin": is_built_in(name),
            "terms": t.map(|t| t.paths()).unwrap_or_default(),
        }),
        vec![json!(name)],
        None,
    ))
}

/// The sections of `docs/vault-format.md` a taxonomy file is written from: taxonomy files with
/// open taxonomies, and the `taxonomy` field kind that uses them.
pub fn taxonomy_format_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        let section = |from: &str, to: &str| {
            let start = VAULT_FORMAT.find(from).unwrap_or(VAULT_FORMAT.len());
            let end = VAULT_FORMAT[start..]
                .find(to)
                .map_or(VAULT_FORMAT.len(), |e| start + e);
            VAULT_FORMAT[start..end].trim().to_owned()
        };
        format!(
            "{}\n\n{}\n",
            section("\n## Taxonomy files\n", "\n## Task files\n"),
            section("\n### `taxonomy`\n", "\n### `reference`\n"),
        )
    })
}

/// `taxonomy_format`: the taxonomy-file vocabulary.
pub(crate) fn format() -> ToolOut {
    out(
        json!({ "format": taxonomy_format_text() }),
        Vec::new(),
        None,
    )
}

/// The `renames` argument, a list of `{from, to}` term paths, as the writer takes it.
fn renames_arg(input: &Value) -> Result<(Vec<Value>, BTreeMap<String, String>), ToolOut> {
    let list = match &input["renames"] {
        Value::Null => return Ok((Vec::new(), BTreeMap::new())),
        Value::Array(a) => a,
        _ => return Err(ToolOut::error("renames must be a list of {from, to}")),
    };
    let mut map = BTreeMap::new();
    let mut shown = Vec::new();
    for r in list {
        let (Some(from), Some(to)) = (r["from"].as_str(), r["to"].as_str()) else {
            return Err(ToolOut::error(
                "each rename must be {from, to}, both term paths",
            ));
        };
        let (from, to) = (from.trim(), to.trim());
        if from.is_empty() || to.is_empty() {
            return Err(ToolOut::error("a rename's from and to must not be empty"));
        }
        if map.insert(from.to_owned(), to.to_owned()).is_some() {
            return Err(ToolOut::error(format!("{from:?} is renamed twice")));
        }
        shown.push(json!({ "from": from, "to": to }));
    }
    Ok((shown, map))
}

/// The renames of a proposal as the writer takes them.
fn renames_of(p: &Value) -> Result<BTreeMap<String, String>, ApplyError> {
    let mut map = BTreeMap::new();
    for r in p["renames"].as_array().into_iter().flatten() {
        match (r["from"].as_str(), r["to"].as_str()) {
            (Some(from), Some(to)) => {
                map.insert(from.to_owned(), to.to_owned());
            }
            _ => return Err(ApplyError::Invalid("a rename has no from or to".into())),
        }
    }
    Ok(map)
}

/// `propose_taxonomy_change`: check the taxonomy file and its renames as the taxonomy write
/// would, without writing them, and hand them to the user.
pub(crate) fn propose_change(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let wcl = input["wcl"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolOut::error("wcl must be the whole taxonomy file as WCL text"))?;
    let (renames, map) = renames_arg(input)?;
    let before = w.read_taxonomy(name).ok();
    let check = w.check_taxonomy(name, wcl, &map).map_err(|e| {
        refused(
            w,
            e,
            "The taxonomy was not proposed. Fix the issues and propose it again.",
        )
    })?;
    let names = vec![json!(name)];
    if before.as_deref() == Some(check.wcl.as_str()) && map.is_empty() {
        return Ok(out(
            json!({
                "proposed": false,
                "message": "The proposed taxonomy file is the same as the current one: there is nothing to apply.",
            }),
            names,
            None,
        ));
    }
    let affected = check.notes.len();
    let content = json!({
        "proposed": true,
        "name": name,
        "exists": check.exists,
        "renames": renames,
        "notes_rewritten": affected,
        "message": if check.exists {
            "The taxonomy change is valid and is shown to the user as a proposal. It has not been written: it awaits the user, who applies or discards it."
        } else {
            "The new taxonomy is valid and is shown to the user as a proposal. It has not been created: it awaits the user, who applies or discards it."
        },
    });
    let proposal = json!({
        "id": call_id,
        "kind": "taxonomy",
        "op": if check.exists { "change" } else { "create" },
        "name": name,
        "exists": check.exists,
        "before": before,
        "after": check.wcl,
        "renames": renames,
        "issues": [],
        "affected": affected,
    });
    Ok(out(content, names, Some(proposal)))
}

/// `propose_taxonomy_delete`: check the taxonomy exists and nothing uses it, without deleting
/// it, and hand the delete to the user.
pub(crate) fn propose_delete(w: &Writer, call_id: &str, input: &Value) -> Result<ToolOut, ToolOut> {
    let name = str_arg(input, "name")?;
    let before = w
        .read_taxonomy(name)
        .map_err(|e| ToolOut::error(e.to_string()))?;
    w.check_delete_taxonomy(name).map_err(|e| {
        refused(
            w,
            e,
            "The delete was not proposed: the types and notes listed use this taxonomy. Change them first.",
        )
    })?;
    let proposal = json!({
        "id": call_id,
        "kind": "taxonomy",
        "op": "delete",
        "name": name,
        "before": before,
    });
    Ok(out(
        json!({
            "proposed": true,
            "name": name,
            "message": "The delete is shown to the user as a proposal. Nothing has been deleted: it awaits the user, who applies or discards it.",
        }),
        vec![json!(name)],
        Some(proposal),
    ))
}

/// Apply a `taxonomy_proposal` as `author` (ai D31), when the taxonomy file is still the
/// proposal's `before`.
pub(crate) fn apply_taxonomy(w: &Writer, author: &Author, p: &Value) -> Result<Value, ApplyError> {
    let name = p["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApplyError::Invalid("the change has no name".into()))?;
    let before = p["before"].as_str();
    if w.read_taxonomy(name).ok().as_deref() != before {
        return Err(ApplyError::Stale(format!(
            "the taxonomy {name} changed since the proposal, so nothing was written"
        )));
    }
    if p["op"] == "delete" {
        w.delete_taxonomy(name)?;
        return Ok(json!({ "op": "taxonomy_delete", "taxonomy_name": name, "notes": 0 }));
    }
    let after = p["after"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApplyError::Invalid("the change has no after".into()))?;
    let renames = renames_of(p)?;
    let (_, saved) = w.put_taxonomy(name, after, &renames, author)?;
    let op = if before.is_some() {
        "taxonomy_change"
    } else {
        "taxonomy_create"
    };
    Ok(json!({ "op": op, "taxonomy_name": name, "notes": saved.len() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxonomy_format_holds_the_taxonomy_file_sections() {
        let t = taxonomy_format_text();
        assert!(t.starts_with("## Taxonomy files"), "{}", &t[..40]);
        assert!(t.contains("### Open taxonomies"));
        assert!(t.contains("### `taxonomy`"));
        assert!(!t.contains("## Task files"));
        assert!(!t.contains("### `reference`"));
    }
}
