use script_core::Vault as _;
use state::Author;
use writer::{Input, Writer};

fn open(dir: &std::path::Path) -> Writer {
    std::fs::create_dir_all(dir.join("types")).unwrap();
    std::fs::write(dir.join("types/email.wcl"), "label = \"Email\"\nfield link { kind = \"reference\" target = \"email\" }\ntemplate default { tab Main { slot title } }\n").unwrap();
    let vault = store::Vault::init(dir).unwrap();
    let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn create(w: &Writer, title: &str) -> writer::Saved {
    w.create(
        "email",
        &Input::Json(serde_json::json!({"fields":{"title":title}})),
        &Author::Session("owner".into()),
    )
    .unwrap()
}

#[test]
fn activation_inventory_is_not_note_date_and_both_legacy_modes_work() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let old = create(&w, "Existing");
    w.activate_agent_safety().unwrap();
    assert!(!w.agent_may_read(&old.id));
    w.state()
        .set_meta("agent_safety.pre_existing_mode", "legacy_allowed")
        .unwrap();
    assert!(w.agent_may_read(&old.id));
    w.state()
        .set_meta("agent_safety.pre_existing_mode", "held")
        .unwrap();
    assert!(!w.agent_may_read(&old.id));
    assert_eq!(
        w.agent_safety(&old.id).unwrap().reason,
        "predates_activation"
    );
    let new = create(&w, "New");
    w.activate_agent_safety().unwrap();
    w.state()
        .set_meta("agent_safety.pre_existing_mode", "legacy_allowed")
        .unwrap();
    assert!(w.agent_may_read(&old.id));
    assert!(!w.agent_may_read(&new.id));
}

#[test]
fn clearance_is_content_bound_and_enqueued_once_even_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let note = create(&w, "First");
    let hash = w.agent_safety(&note.id).unwrap().fingerprint;
    assert!(
        w.finish_agent_safety(&note.id, &hash, "allowed", "allow")
            .unwrap()
    );
    assert!(w.agent_may_read(&note.id));
    assert_eq!(
        w.state().take_agent_safety_clearances().unwrap(),
        vec![note.id.clone()]
    );
    assert!(
        w.finish_agent_safety(&note.id, &hash, "released", "user_released")
            .unwrap()
    );
    assert!(w.state().take_agent_safety_clearances().unwrap().is_empty());
    w.update(
        &note.id,
        &Input::Json(serde_json::json!({"fields":{"title":"Changed"}})),
        Some(&note.version),
        &Author::Session("owner".into()),
    )
    .unwrap();
    assert!(!w.agent_may_read(&note.id));
    assert!(
        !w.finish_agent_safety(&note.id, &hash, "allowed", "allow")
            .unwrap()
    );
    drop(w);
    let vault = store::Vault::open(dir.path()).unwrap();
    let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = Writer::open(vault, state).unwrap();
    assert!(!w.agent_may_read(&note.id));
    assert!(w.state().take_agent_safety_clearances().unwrap().is_empty());
}

#[test]
fn user_release_wins_over_inflight_decision() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let note = create(&w, "Pending");
    let hash = w.agent_safety(&note.id).unwrap().fingerprint;
    w.finish_agent_safety(&note.id, &hash, "released", "user_released")
        .unwrap();
    assert!(
        !w.finish_agent_safety(&note.id, &hash, "held", "scam")
            .unwrap()
    );
    assert_eq!(w.agent_safety(&note.id).unwrap().status, "released");
}

#[test]
fn agent_reads_filter_roots_and_reference_traversal_but_user_reads_remain() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let hidden = create(&w, "PRIVATE_INJECTION");
    let visible = w
        .create(
            "email",
            &Input::Json(serde_json::json!({"fields":{"title":"Ordinary", "link":hidden.id}})),
            &Author::Session("owner".into()),
        )
        .unwrap();
    let hash = w.agent_safety(&visible.id).unwrap().fingerprint;
    w.finish_agent_safety(&visible.id, &hash, "allowed", "allow")
        .unwrap();
    assert!(
        w.get(&hidden.id)
            .unwrap()
            .to_string()
            .contains("PRIVATE_INJECTION")
    );
    let _acting = writer::acting_as(&Author::Agent("test".into()));
    assert!(w.get(&hidden.id).is_err());
    let rows = w
        .query_page("from email select title, link.title", 100, None)
        .unwrap();
    assert!(!rows.to_string().contains("PRIVATE_INJECTION"), "{rows}");
    assert_eq!(rows["total"], 1);
    assert!(
        w.search("PRIVATE_INJECTION", 10, script_core::Archived::Include)
            .unwrap()
            .is_empty()
    );
    assert!(w.backlinks(&hidden.id).is_err());
}

#[test]
fn forged_annotation_cannot_clear_missing_or_failed_decision() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let note = w
        .create(
            "email",
            &Input::Json(
                serde_json::json!({"fields":{"title":"Forged", "agent_safety_reason":"allow"}}),
            ),
            &Author::Session("owner".into()),
        )
        .unwrap();
    assert!(!w.agent_may_read(&note.id));
    let hash = w.agent_safety(&note.id).unwrap().fingerprint;
    w.finish_agent_safety(&note.id, &hash, "held", "decision_failed")
        .unwrap();
    assert!(!w.agent_may_read(&note.id));
}
