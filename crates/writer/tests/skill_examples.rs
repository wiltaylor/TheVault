//! The examples of the shipped `vault-notes` skill (TV-34): its taxonomy and type files are
//! written to a fresh vault, and each of its notes is validated as a create, so the skill never
//! teaches a note or a type the vault refuses.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use state::{Author, State};
use store::Vault;
use writer::{Input, Op, SHIPPED_FILES, Upkeep, Writer};

const SKILL: &str = "skills/vault-notes/SKILL.md";

/// The ids the skill's examples name, made here so they exist.
const PERSON: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const PROJECT: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";
const IMAGE: &str = "01JB9A0C3D4E5F6G7H8J9K0M1N";

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = Writer::open(vault, state).unwrap();
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    w
}

fn author() -> Author {
    Author::Session("test".into())
}

/// Each fenced block of `text` whose info string is `<lang> <kind>:<name>`, as
/// `(kind, name, body)`, in order.
fn examples(text: &str, lang: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(info) = line.strip_prefix("```").and_then(|i| i.strip_prefix(lang)) else {
            continue;
        };
        let Some((kind, name)) = info.trim().split_once(':') else {
            continue;
        };
        let body: Vec<&str> = lines.by_ref().take_while(|l| *l != "```").collect();
        out.push((kind.to_owned(), name.to_owned(), body.join("\n")));
    }
    out
}

#[test]
fn every_type_and_note_in_the_vault_notes_skill_is_valid() {
    let source = SHIPPED_FILES
        .iter()
        .find(|f| f.path == SKILL)
        .expect("vault-notes is shipped")
        .source;
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());

    // The definitions, in the skill's order.
    let defs = examples(source, "wcl");
    assert!(defs.iter().any(|(k, ..)| k == "type"), "no type examples");
    for (kind, name, body) in &defs {
        match kind.as_str() {
            "taxonomy" => {
                w.put_taxonomy(name, body, &BTreeMap::new(), &author())
                    .unwrap_or_else(|e| panic!("taxonomy {name}: {e}"));
            }
            "type" => {
                w.put_type(name, body)
                    .unwrap_or_else(|e| panic!("type {name}: {e}"));
            }
            other => panic!("unknown wcl example kind {other}"),
        }
    }
    // The two-way `project` field gave project its `tasks`.
    assert!(
        w.schema()
            .type_def("project")
            .unwrap()
            .field("tasks")
            .is_some()
    );

    // What the notes name.
    let make = |id: &str, ty: &str, title: &str| {
        w.create_with_id(
            id,
            ty,
            &Input::Json(json!({ "fields": { "title": title } })),
            &author(),
        )
        .unwrap_or_else(|e| panic!("{ty} {id}: {e}"));
    };
    make(PERSON, "person", "Ann Lee");
    make(PROJECT, "project", "Website");
    w.put_asset_with_id(IMAGE, "plan.png", b"\x89PNG\r\n\x1a\n".to_vec())
        .unwrap();

    let notes = examples(source, "json");
    let mut checked = 0;
    for (kind, ty, body) in notes {
        assert_eq!(kind, "note", "unknown json example kind {kind}");
        let note: Value =
            serde_json::from_str(&body).unwrap_or_else(|e| panic!("note:{ty} is not JSON: {e}"));
        let report = w
            .batch(
                vec![Op::Create {
                    type_name: ty.clone(),
                    input: Input::Json(note),
                }],
                true,
                &author(),
            )
            .unwrap();
        assert!(
            report.ok,
            "note:{ty} is refused: {:?} {:?}",
            report.items, report.error
        );
        checked += 1;
    }
    assert!(checked >= 2, "{checked} note examples");
}
