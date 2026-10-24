//! The built-in `agent_memory` type in new and existing vaults (agents G21).

use std::path::Path;

use state::State;
use store::Vault;
use writer::{AGENT_MEMORY_TYPE, Upkeep, Writer};

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn generation(root: &Path) -> u64 {
    std::fs::read_to_string(root.join(".vault/generation"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0)
}

#[test]
fn a_new_vault_gets_the_agent_memory_type_in_its_one_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    let before = generation(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(generation(root), before + 1);
    assert!(done.types_added.iter().any(|t| t == AGENT_MEMORY_TYPE));
    let schema = w.schema();
    let ty = schema.type_def(AGENT_MEMORY_TYPE).unwrap();
    assert_eq!(ty.label, "Agent memory");
    assert_eq!(ty.plural, "Agent memories");
    for f in ["agent", "body"] {
        assert!(ty.field(f).is_some(), "{f}");
    }
    assert_eq!(ty.field("body").unwrap().kind, schema::FieldKind::Content);
    assert!(w.is_built_in(AGENT_MEMORY_TYPE));
    assert!(writer::matches_built_in(ty));
    let page = ty.list_page.as_ref().unwrap();
    assert_eq!(page.sort, Some(("updated".to_owned(), true)));
    assert_eq!(page.view.as_deref(), Some("list"));
}

#[test]
fn an_existing_vault_gets_the_agent_memory_type_on_its_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    // A vault from before agent memory.
    std::fs::remove_file(root.join("types/agent_memory.wcl")).unwrap();
    let w = open(root);
    assert!(w.schema().type_def(AGENT_MEMORY_TYPE).is_none());
    let before = generation(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(done.types_added, [AGENT_MEMORY_TYPE]);
    assert_eq!(generation(root), before + 1);
    assert!(w.schema().type_def(AGENT_MEMORY_TYPE).is_some());
    assert!(w.is_built_in(AGENT_MEMORY_TYPE));
}
