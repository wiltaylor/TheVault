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
    assert!(!w.state().held_agent_note_ids().unwrap().contains(&old.id));
    w.state()
        .set_meta("agent_safety.pre_existing_mode", "held")
        .unwrap();
    assert!(!w.agent_may_read(&old.id));
    assert!(w.state().held_agent_note_ids().unwrap().contains(&old.id));
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
        w.state().agent_safety_clearances().unwrap(),
        vec![note.id.clone()]
    );
    assert_eq!(
        w.state().agent_safety_clearances().unwrap(),
        vec![note.id.clone()]
    );
    w.state()
        .acknowledge_agent_safety_clearance(&note.id)
        .unwrap();
    assert!(
        w.finish_agent_safety(&note.id, &hash, "released", "user_released")
            .unwrap()
    );
    assert!(w.state().agent_safety_clearances().unwrap().is_empty());
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
    assert!(w.state().agent_safety_clearances().unwrap().is_empty());
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

#[test]
fn review_routine_workflow_writes_keep_clearance() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let note = create(&w, "Incoming");
    let hash = w.agent_safety(&note.id).unwrap().fingerprint;
    w.finish_agent_safety(&note.id, &hash, "allowed", "allow")
        .unwrap();
    for fields in [
        serde_json::json!({"title":"Incoming", "labels":["system/INBOX"]}),
        serde_json::json!({"title":"Incoming", "processed":true}),
        serde_json::json!({"title":"Incoming", "images_pending":false}),
    ] {
        let version = w.read(&note.id).unwrap().version;
        w.update(
            &note.id,
            &Input::Json(serde_json::json!({"fields":fields})),
            Some(&version),
            &Author::Session("owner".into()),
        )
        .unwrap();
        assert!(
            w.agent_may_read(&note.id),
            "workflow write invalidated clearance: {fields}"
        );
    }
    let version = w.read(&note.id).unwrap().version;
    w.update(
        &note.id,
        &Input::Json(serde_json::json!({"fields":{"title":"Changed content"}})),
        Some(&version),
        &Author::Session("owner".into()),
    )
    .unwrap();
    assert_eq!(w.agent_safety(&note.id).unwrap().reason, "content_changed");
}

#[test]
fn review_legacy_release_never_replays_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let note = create(&w, "Already arrived");
    w.activate_agent_safety().unwrap();
    let hash = w.agent_safety(&note.id).unwrap().fingerprint;
    assert!(
        w.release_agent_safety(&note.id, &hash, &Author::Session("owner".into()))
            .unwrap()
    );
    assert!(w.state().agent_safety_clearances().unwrap().is_empty());
}

#[test]
fn review_invited_series_is_in_scope() {
    let doc = schema::Doc::parse("title = \"Untrusted recurring invitation\"\norganiser = \"outside@example.test\"\ncalendar = \"me@example.test\"\n", "series.wcl").unwrap();
    assert!(writer::agent_safety::incoming("event_series", &doc));
}

#[test]
fn review_sync_snapshot_is_not_classified_content() {
    let a = schema::Doc::parse(
        "title = \"Invitation\"\ndict snapshot { title = \"Before sync\" }\n",
        "event.wcl",
    )
    .unwrap();
    let b = schema::Doc::parse(
        "title = \"Invitation\"\ndict snapshot { title = \"After sync\" }\n",
        "event.wcl",
    )
    .unwrap();
    assert_eq!(
        writer::agent_safety::fingerprint("event", &a),
        writer::agent_safety::fingerprint("event", &b)
    );
    assert!(
        !writer::agent_safety::classified_content(&a)
            .to_string()
            .contains("Before sync")
    );
}

#[test]
fn review_index_excludes_many_held_notes_without_reading_their_files() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let held: Vec<_> = (0..64).map(|n| create(&w, &format!("Held {n}"))).collect();
    let visible = create(&w, "Allowed");
    let hash = w.agent_safety(&visible.id).unwrap().fingerprint;
    w.finish_agent_safety(&visible.id, &hash, "allowed", "allow")
        .unwrap();
    for note in &held {
        // A per-note read/parse would fail: queries must use the committed index instead.
        std::fs::write(
            dir.path().join(format!("notes/email/{}.wcl", note.id)),
            [0xff],
        )
        .unwrap();
    }
    assert_eq!(w.state().held_agent_note_ids().unwrap().len(), 64);
    let _acting = writer::acting_as(&Author::Agent("test".into()));
    let rows = w.query_page("from email select title", 100, None).unwrap();
    assert_eq!(rows["total"], 1);
    assert!(rows.to_string().contains("Allowed"));
}

#[test]
fn review_agent_created_note_has_no_clearance_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let note = w
        .create(
            "email",
            &Input::Json(serde_json::json!({"fields":{"title":"Agent wrote this"}})),
            &Author::Agent("test".into()),
        )
        .unwrap();
    let hash = w.agent_safety(&note.id).unwrap().fingerprint;
    w.finish_agent_safety(&note.id, &hash, "allowed", "allow")
        .unwrap();
    assert!(w.state().agent_safety_clearances().unwrap().is_empty());
}

#[test]
fn review_interrupted_pending_is_held_without_retry_on_restart() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let note = create(&w, "Pending");
    let read = w.read(&note.id).unwrap();
    w.register_agent_safety(&note.id, &read.type_name, &read.doc)
        .unwrap();
    assert_eq!(w.agent_safety(&note.id).unwrap().status, "pending");
    drop(w);
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let safety = w.agent_safety(&note.id).unwrap();
    assert_eq!(safety.status, "held");
    assert_eq!(safety.reason, "unclassified");
    assert!(w.state().agent_safety_clearances().unwrap().is_empty());
}

#[test]
fn round3_agent_write_cannot_return_or_change_held_note_reached_through_ref() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let hidden = create(&w, "PRIVATE_INJECTION_TEXT");
    let visible = w.create("email", &Input::Json(serde_json::json!({"fields":{"title":"Reviewed", "link":hidden.id}})), &Author::Session("owner".into())).unwrap();
    let hash = w.agent_safety(&visible.id).unwrap().fingerprint;
    w.finish_agent_safety(&visible.id, &hash, "allowed", "allow").unwrap();
    let actor = Author::Agent("reader".into());
    let _acting = writer::acting_as(&actor);
    let reference = w.get(&visible.id).unwrap()["note"]["fields"]["link"].as_str().unwrap().to_owned();
    assert!(w.get(&reference).is_err());
    let result = script_core::Vault::set_archived(&w, &reference, true, &actor);
    assert!(result.is_err(), "agent write leaked held note: {result:?}");
    assert_eq!(w.read(&hidden.id).unwrap().version, hidden.version);
}

#[test]
fn round3_derived_email_thread_title_is_held_with_first_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::write(dir.path().join("types/email_thread.wcl"), "label = \"Thread\"\ntemplate default { tab Main { slot title } }\n").unwrap();
    let w = open(dir.path());
    w.activate_agent_safety().unwrap();
    let actor = Author::Session("owner".into());
    let thread = w.create("email_thread", &Input::Json(serde_json::json!({"fields":{"title":"PRIVATE_SUBJECT"}})), &actor).unwrap();
    w.create("email", &Input::Json(serde_json::json!({"fields":{"title":"PRIVATE_SUBJECT", "thread":thread.id}})), &actor).unwrap();
    assert!(!w.agent_may_read(&thread.id), "derived unclassified subject reached an agent");
}
