//! The starter pack `vault init --starter` adds: the Task, Project, Person and Note types, the
//! Status and Tags taxonomies, a few example notes and a dashboard note with kanban and calendar
//! views (D37). Tags is open, so a note's new tag adds itself to it. It also holds the default
//! device scripts and the `devices/android.wcl` that wires them, for the user to edit (D15).
//!
//! Definitions are written as they are. Notes go through `writer`, so they are validated, stamped
//! and recorded in `state.db` like any other write. A note file may hold `{{id:<key>}}`, the id
//! of a note created before it, and `{{date:<+n|-n>}}`, today's date moved by `n` days.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use state::{Author, State};
use store::{Change, Vault};
use writer::{Input, Writer};

use crate::Fail;

const TAXONOMIES: &[(&str, &str)] = &[
    ("status", include_str!("../starter/taxonomies/status.wcl")),
    ("tags", include_str!("../starter/taxonomies/tags.wcl")),
];

const TYPES: &[(&str, &str)] = &[
    ("person", include_str!("../starter/types/person.wcl")),
    ("project", include_str!("../starter/types/project.wcl")),
    ("task", include_str!("../starter/types/task.wcl")),
    ("note", include_str!("../starter/types/note.wcl")),
];

/// `(key, type, source)`, in creation order: a note names only notes above it.
const NOTES: &[(&str, &str, &str)] = &[
    ("alex", "person", include_str!("../starter/notes/alex.wcl")),
    (
        "project",
        "project",
        include_str!("../starter/notes/project.wcl"),
    ),
    (
        "task_install",
        "task",
        include_str!("../starter/notes/task_install.wcl"),
    ),
    (
        "task_tour",
        "task",
        include_str!("../starter/notes/task_tour.wcl"),
    ),
    (
        "task_import",
        "task",
        include_str!("../starter/notes/task_import.wcl"),
    ),
    (
        "task_type",
        "task",
        include_str!("../starter/notes/task_type.wcl"),
    ),
    (
        "dashboard",
        "note",
        include_str!("../starter/notes/dashboard.wcl"),
    ),
    (
        "welcome",
        "note",
        include_str!("../starter/notes/welcome.wcl"),
    ),
];

/// Write the starter pack into the new, empty vault at `vault`. Returns what it added.
pub fn install(vault: Vault, state: State) -> Result<Value, Fail> {
    let mut changes = Vec::new();
    for (name, source) in TAXONOMIES {
        changes.push(definition(&format!("taxonomies/{name}.wcl"), source)?);
    }
    for (name, source) in TYPES {
        changes.push(definition(&writer::type_path(name), source)?);
    }
    for (name, source) in writer::DEFAULT_DEVICE_SCRIPTS {
        changes.push(Change::write(
            format!("scripts/{name}.ws"),
            source.as_bytes(),
        ));
    }
    let android = writer::device_file_path("android");
    changes.push(definition(&android, writer::DEFAULT_ANDROID_DEVICE_FILE)?);
    vault.commit(changes).map_err(Fail::from_display("store"))?;

    let writer = Writer::open(vault, state).map_err(Fail::from_display("starter"))?;
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .map_err(Fail::from_display("starter"))?;
    let author = Author::Session(writer::ADOPTED_BY.to_owned());
    let today = days_since_epoch();
    let mut ids: BTreeMap<&str, String> = BTreeMap::new();
    let mut notes = Vec::new();
    for (key, type_name, source) in NOTES {
        let text = render(source, &ids, today)?;
        let saved = writer
            .create(type_name, &Input::Wcl(text), &author)
            .map_err(|e| Fail::new("starter", format!("starter note {key}: {e}")))?;
        notes.push(json!({ "id": saved.id, "type": type_name }));
        ids.insert(key, saved.id);
    }
    Ok(json!({
        "taxonomies": TAXONOMIES.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "types": TYPES.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "scripts": writer::DEFAULT_DEVICE_SCRIPTS.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "device_files": [android],
        "notes": notes,
        "dashboard": ids["dashboard"],
    }))
}

fn definition(path: &str, source: &str) -> Result<Change, Fail> {
    let text = store::canonical(source, path).map_err(Fail::from_display("starter"))?;
    Ok(Change::write(path, text))
}

/// Replace every `{{id:<key>}}` and `{{date:<offset>}}` in `source`.
fn render(source: &str, ids: &BTreeMap<&str, String>, today: i64) -> Result<String, Fail> {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| Fail::new("starter", "an unclosed {{ in a starter note"))?;
        let token = &after[..end];
        let value = if let Some(key) = token.strip_prefix("id:") {
            ids.get(key)
                .cloned()
                .ok_or_else(|| Fail::new("starter", format!("no starter note {key:?} yet")))?
        } else if let Some(offset) = token.strip_prefix("date:") {
            let n: i64 = offset
                .trim_start_matches('+')
                .parse()
                .map_err(|_| Fail::new("starter", format!("bad date offset {offset:?}")))?;
            civil_date(today + n)
        } else {
            return Err(Fail::new(
                "starter",
                format!("unknown token {{{{{token}}}}}"),
            ));
        };
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn days_since_epoch() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    i64::try_from(secs / 86_400).unwrap_or(0)
}

/// `YYYY-MM-DD` of the UTC day `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(-1), "1969-12-31");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_739), "2026-10-13");
    }

    #[test]
    fn render_fills_ids_and_dates() {
        let mut ids = BTreeMap::new();
        ids.insert("a", "ID1".to_owned());
        let out = render("x = \"{{id:a}}\"\nd = \"{{date:-1}}\"\n", &ids, 1).unwrap();
        assert_eq!(out, "x = \"ID1\"\nd = \"1970-01-01\"\n");
        assert!(render("{{id:b}}", &ids, 0).is_err());
    }
}
