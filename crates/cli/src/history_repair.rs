use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Write};
use std::path::{Component, Path};

use serde_json::{Value, json};

use crate::{Fail, local::state_db};

pub fn check(vault: &Path) -> Result<Value, Fail> {
    let corrupt =
        state::history_repair::check(&state_db(vault)).map_err(Fail::from_display("state"))?;
    let out = json!({"ok": corrupt.is_empty(), "corrupt": corrupt});
    if corrupt.is_empty() {
        Ok(out)
    } else {
        Err(Fail::body(out))
    }
}

pub fn repair(
    vault: &Path,
    id: Option<String>,
    all: bool,
    yes: Vec<String>,
) -> Result<Value, Fail> {
    // Do not open State/Writer: recovery cannot run on a corrupt head, and opening
    // State can migrate or prune even before the owner has confirmed anything.
    if !state_db(vault).is_file() {
        return Err(Fail::new("not_a_vault", "missing .vault/state.db"));
    }
    let _owner = store::lock_owner(vault).map_err(Fail::from_display("vault_busy"))?;
    let corrupt =
        state::history_repair::check(&state_db(vault)).map_err(Fail::from_display("state"))?;
    let selected: Vec<_> = corrupt
        .into_iter()
        .filter(|r| all || id.as_deref() == Some(&r.note_id))
        .collect();
    let ids: BTreeSet<_> = selected.iter().map(|r| r.note_id.clone()).collect();
    if ids.is_empty() {
        return Ok(json!({"ok": true, "actions": []}));
    }
    if yes.is_empty() {
        if !std::io::stdin().is_terminal() {
            return Err(Fail::new(
                "confirmation_required",
                "pass --yes with the exact comma-separated note IDs, or confirm on a terminal",
            ));
        }
        eprint!(
            "Repair corrupt history for {}? Type yes: ",
            ids.iter().cloned().collect::<Vec<_>>().join(",")
        );
        std::io::stderr()
            .flush()
            .map_err(Fail::from_display("io"))?;
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(Fail::from_display("io"))?;
        if answer.trim() != "yes" {
            return Err(Fail::new(
                "confirmation_required",
                "history repair cancelled",
            ));
        }
    } else if yes.into_iter().collect::<BTreeSet<_>>() != ids {
        return Err(Fail::new(
            "confirmation_required",
            "--yes must name exactly the affected note IDs",
        ));
    }
    let root = vault.canonicalize().map_err(Fail::from_display("io"))?;
    let mut contents = BTreeMap::new();
    for row in selected {
        let content = if row.head {
            for part in [&row.note_id, &row.note_type] {
                if !matches!(
                    Path::new(part).components().collect::<Vec<_>>().as_slice(),
                    [Component::Normal(_)]
                ) {
                    return Err(Fail::new(
                        "invalid_path",
                        "invalid note ID or type in history",
                    ));
                }
            }
            let path = root
                .join("notes")
                .join(&row.note_type)
                .join(format!("{}.wcl", row.note_id));
            let path = path.canonicalize().map_err(Fail::from_display("io"))?;
            if !path.starts_with(root.join("notes")) {
                return Err(Fail::new(
                    "invalid_path",
                    "note file is outside the vault notes folder",
                ));
            }
            Some(std::fs::read_to_string(path).map_err(Fail::from_display("io"))?)
        } else {
            None
        };
        let entry = contents.entry(row.note_id).or_insert(None);
        if content.is_some() {
            *entry = content;
        }
    }
    let actions = state::history_repair::repair(&state_db(vault), &contents)
        .map_err(Fail::from_display("state"))?;
    Ok(json!({"ok": true, "actions": actions}))
}
