use state::{Author, State};
use store::{Change, Vault};
use writer::{Error, Input, Writer};

fn setup() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    std::fs::write(
        dir.path().join("types/item.wcl"),
        "label = \"Item\"\ntemplate default { tab \"Main\" { slot \"title\" } }\n",
    )
    .unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let writer = Writer::open(vault, state).unwrap();
    (dir, writer)
}

#[test]
fn upgrade_holds_queued_legacy_work_once_with_actionable_subjects() {
    let (dir, writer) = setup();
    let handler = writer
        .state()
        .queue_jobs(&[
            serde_json::json!({"script":"review", "type":"item", "id":"legacy-note"}).to_string(),
        ])
        .unwrap()[0];
    let trigger = writer
        .state()
        .enqueue_trigger("legacy-agent", "nightly", Some("legacy-note"), "{}", 0)
        .unwrap()
        .unwrap();
    drop(writer);
    let conn = rusqlite::Connection::open(dir.path().join(".vault/state.db")).unwrap();
    conn.execute_batch("DROP TABLE handler_provenance; DROP TABLE outbox_provenance; DROP TABLE trigger_provenance; PRAGMA user_version=32;").unwrap();
    drop(conn);
    for _ in 0..2 {
        let vault = Vault::open(dir.path()).unwrap();
        let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
        let writer = Writer::open(vault, state).unwrap();
        assert!(
            writer
                .state()
                .handler_provenance(handler)
                .unwrap()
                .is_none()
        );
        assert!(
            writer
                .state()
                .trigger_provenance(trigger)
                .unwrap()
                .is_empty()
        );
        for subject in [
            writer.state().handler_recovery_subject(handler).unwrap(),
            writer.state().trigger_recovery_subject(trigger).unwrap(),
        ] {
            for _ in 0..3 {
                assert!(writer.recover_run_context_for("", &subject).is_err());
            }
        }
        let activity = writer
            .state()
            .activity(&state::ActivityFilter::default())
            .unwrap();
        let held: Vec<_> = activity
            .items
            .iter()
            .filter(|a| a.kind == "run.recovery_unavailable")
            .collect();
        assert_eq!(held.len(), 2);
        assert!(held.iter().all(|a| a.summary.contains("legacy-note")));
        assert!(held.iter().any(
            |a| a.summary.contains("review") && a.summary.contains("editing the note as owner")
        ));
        assert!(
            held.iter().any(|a| a.summary.contains("legacy-agent")
                && a.summary.contains("manual run or backfill"))
        );
        assert!(
            writer
                .state()
                .queued_jobs()
                .unwrap()
                .iter()
                .any(|j| j.id == handler)
        );
    }
}

#[test]
fn failed_recovery_audit_releases_each_new_lease() {
    let (dir, w) = setup();
    let context = script_core::RunContext::default();
    let id = context.persist(w.state()).unwrap();
    drop(context);
    w.state().set_automation_stopped(true).unwrap();
    w.state().set_automation_stopped(false).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join(".vault/state.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON activity BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;").unwrap();
    for _ in 0..3 {
        assert!(script_core::RunContext::recover(w.state(), &id).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM run_provenance_leases", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(w.state().load_run_provenance(&id).is_sealed());
    }
    conn.execute_batch("DROP TRIGGER refuse_audit").unwrap();
    let recovered = script_core::RunContext::recover(w.state(), &id).unwrap();
    assert!(
        w.state()
            .automation_commit(recovered.stop_generation().unwrap())
            .is_err()
    );
    drop(recovered);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM run_provenance_leases", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn shared_registry_reuses_joined_aliases_and_seals_after_last_queue_reference() {
    let (_dir, w) = setup();
    let a = script_core::RunContext::default();
    let b = script_core::RunContext::default();
    let a_id = a.persist(w.state()).unwrap();
    let b_id = b.persist(w.state()).unwrap();
    a.merge(&b);
    drop(a);
    drop(b);
    let jobs = w
        .state()
        .queue_jobs_with_provenance(&["job".into()], &b_id)
        .unwrap();
    let a = w.recover_run_context(&a_id).unwrap();
    let b = w.recover_run_context(&b_id).unwrap();
    assert_eq!(a, b);
    assert!(!a.is_restricted());
    assert!(
        w.state()
            .automation_commit(a.stop_generation().unwrap())
            .is_ok()
    );
    a.restrict();
    drop(a);
    drop(b);
    w.prune_run_contexts();
    assert!(!w.state().load_run_provenance(&a_id).is_sealed());
    w.state().finish_job(jobs[0], Ok(()), "").unwrap();
    w.prune_run_contexts();
    assert!(w.state().load_run_provenance(&a_id).is_sealed());
    assert!(w.state().load_run_provenance(&b_id).is_restricted());
}

#[test]
fn outbox_reference_failure_aborts_before_any_file_commit() {
    let (dir, writer) = setup();
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let conn = rusqlite::Connection::open(dir.path().join(".vault/state.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_outbox_reference BEFORE INSERT ON outbox_provenance BEGIN SELECT RAISE(FAIL,'injected reference failure'); END;").unwrap();
    let context = script_core::RunContext::default();
    context.restrict();
    let result = context.scope(|| writer.batch(vec![
        writer::Op::Create { type_name: "item".into(), input: Input::Wcl("title = \"Unrelated\"\n".into()) },
        writer::Op::Create { type_name: "outbox".into(), input: Input::Json(serde_json::json!({"fields": {
            "title":"Not committed", "kind":"email", "status":"sending", "to":["ann@example.com"]
        }})) },
    ], false, &Author::Session("owner".into())));
    assert!(!result.unwrap().ok);
    assert!(writer.list(Some("outbox")).unwrap().is_empty());
    assert!(writer.list(Some("item")).unwrap().is_empty());
    conn.execute_batch("DROP TRIGGER refuse_outbox_reference;")
        .unwrap();
    drop(conn);
    drop(context);
    drop(writer);
    let vault = Vault::open(dir.path()).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let writer = Writer::open(vault, state).unwrap();
    assert!(writer.list(Some("outbox")).unwrap().is_empty());
    assert!(writer.list(Some("item")).unwrap().is_empty());
}

#[test]
fn first_outbox_update_after_restart_publishes_before_post_commit_failure() {
    let (dir, writer) = setup();
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let input = Input::Json(
        serde_json::json!({"fields": {"title":"Old", "kind":"email", "status":"draft", "to":["ann@example.com"]}}),
    );
    let item = writer
        .create("outbox", &input, &Author::Session("owner".into()))
        .unwrap();
    let old = writer.state().outbox_provenance(&item.id).unwrap()[0].clone();
    drop(writer);
    let vault = Vault::open(dir.path()).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    assert!(state.load_run_provenance(&old).is_sealed());
    let writer = Writer::open(vault, state).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join(".vault/state.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_version BEFORE INSERT ON versions BEGIN SELECT RAISE(ABORT,'injected post-file failure'); END;").unwrap();
    let update = script_core::RunContext::default();
    update.restrict();
    let changed = Input::Json(
        serde_json::json!({"fields": {"title":"New", "kind":"email", "status":"draft", "to":["ann@example.com"]}}),
    );
    let error = update
        .scope(|| {
            writer.update(
                &item.id,
                &changed,
                Some(&item.version),
                &Author::Token("automation-test".into()),
            )
        })
        .unwrap_err();
    assert!(
        std::fs::read_to_string(dir.path().join(format!("notes/outbox/{}.wcl", item.id)))
            .unwrap()
            .contains("New"),
        "unexpected pre-file error: {error}"
    );
    let ids = writer.state().outbox_provenance(&item.id).unwrap();
    assert!(!ids.is_empty());
    assert!(!writer.state().load_run_provenance(&old).is_sealed());
    // The new reference is already active at the failed post-file boundary.
    assert!(
        ids.iter()
            .any(|id| writer.state().load_run_provenance(id).is_restricted())
    );
    drop(update);
    drop(writer);
    let state = State::open(&dir.path().join(".vault/state.db"), None).unwrap();
    assert!(
        ids.iter()
            .all(|id| state.load_run_provenance(id).is_restricted())
    );
}

#[test]
fn outbox_owner_action_replaces_stale_admission_but_automation_cannot() {
    let (_dir, writer) = setup();
    writer
        .upgrade_shipped_defaults(writer::Upkeep::Home)
        .unwrap();
    let input = Input::Json(
        serde_json::json!({"fields": {"title":"Held", "kind":"email", "status":"draft", "to":["ann@example.com"]}}),
    );
    let item = writer
        .create("outbox", &input, &Author::Session("owner".into()))
        .unwrap();
    writer.state().set_automation_stopped(true).unwrap();
    writer.state().set_automation_stopped(false).unwrap();
    for _ in 0..3 {
        assert!(writer.outbox_run_context(&item.id).is_err());
    }
    let audits = writer
        .state()
        .activity(&state::ActivityFilter::default())
        .unwrap();
    assert_eq!(
        audits
            .items
            .iter()
            .filter(|a| a.kind == "run.recovery_stale" && a.summary.contains(&item.id))
            .count(),
        1
    );
    let automation = script_core::RunContext::default();
    let ready = Input::Json(
        serde_json::json!({"fields": {"title":"Held", "kind":"email", "status":"ready", "to":["ann@example.com"]}}),
    );
    let mut automated = writer::doc_to_json(&writer.read(&item.id).unwrap().doc);
    automated["fields"]["error"] = serde_json::json!("automated attempt");
    assert!(matches!(
        automation.scope(|| writer.update(
            &item.id,
            &Input::Json(automated),
            Some(&item.version),
            &Author::Handler("test".into())
        )),
        Err(Error::State(state::Error::AutomationRefused))
    ));
    let fresh = script_core::RunContext::default();
    fresh
        .scope(|| {
            writer.update(
                &item.id,
                &ready,
                Some(&item.version),
                &Author::Session("owner".into()),
            )
        })
        .unwrap();
    let recovered = writer.outbox_run_context(&item.id).unwrap();
    assert_eq!(recovered, fresh);
    recovered
        .scope_recovered(|| writer.put_script("sent_after_resume", "fn run() {}\n"))
        .unwrap();
    assert_eq!(writer.state().outbox_provenance(&item.id).unwrap().len(), 1);
}

#[test]
fn run_context_bridges_admission_at_note_and_definition_commits() {
    let (dir, w) = setup();
    let context = script_core::RunContext::default();
    let old = w.state().admit_automation().unwrap();
    context.capture_stop_generation(old);
    context.restrict();
    // Restriction alone adds no refusal, and a live admission still commits.
    context
        .scope(|| w.put_script("live", "fn run() {}\n"))
        .unwrap();
    w.state().set_automation_stopped(true).unwrap();
    w.state().set_automation_stopped(false).unwrap();
    let fresh = w.state().admit_automation().unwrap();
    let child = script_core::RunContext::default();
    child.capture_stop_generation(fresh);
    let result = context.scope(|| {
        child.scope(|| w.with_automation(fresh, || w.put_script("stale", "fn run() {}\n")))
    });
    assert!(matches!(
        result,
        Err(Error::State(state::Error::AutomationRefused))
    ));
    assert!(!dir.path().join("scripts/stale.ws").exists());
    assert_eq!(child.stop_generation(), Some(old));
    assert!(child.is_restricted());
    let result = child.scope(|| {
        w.create(
            "item",
            &Input::Wcl("title = \"Old\"\n".into()),
            &Author::Session("test".into()),
        )
    });
    assert!(matches!(
        result,
        Err(Error::State(state::Error::AutomationRefused))
    ));
    // No context or credential is installed after the synchronous closure.
    assert!(script_core::RunContext::current().is_none());
    w.put_script("owner", "fn run() {}\n").unwrap();
}

#[test]
fn empty_run_context_preserves_unscoped_writer_behavior() {
    let (_dir, w) = setup();
    w.state().set_automation_stopped(true).unwrap();
    let context = script_core::RunContext::default();
    context
        .scope(|| w.put_script("unscoped", "fn run() {}\n"))
        .unwrap();
    assert_eq!(context.stop_generation(), None);
    assert!(!context.is_restricted());
}

#[test]
fn stopped_and_stale_commits_leave_files_history_and_index_unchanged() {
    let (dir, w) = setup();
    let author = Author::Session("test".into());
    let input = Input::Wcl("title = \"Before\"\n".into());
    let note = w.create("item", &input, &author).unwrap();
    let path = dir.path().join(format!("notes/item/{}.wcl", note.id));
    let before = std::fs::read(&path).unwrap();
    let history = w.history(&note.id).unwrap().len();
    let index_generation = w.read_index(|i, _| i.generation().unwrap());
    let definitions = w.definition_changes();
    let generation = w.state().admit_automation().unwrap();
    w.state().set_automation_stopped(true).unwrap();
    for stopped in [true, false] {
        if !stopped {
            w.state().set_automation_stopped(false).unwrap();
        }
        w.with_automation(generation, || {
            assert!(matches!(
                w.update(
                    &note.id,
                    &Input::Wcl("title = \"After\"\n".into()),
                    Some(&note.version),
                    &author
                ),
                Err(Error::State(state::Error::AutomationRefused))
            ));
            assert!(matches!(
                w.create("item", &input, &author),
                Err(Error::State(state::Error::AutomationRefused))
            ));
            let result: Result<(), Error> =
                w.commit_checked(|_| Ok(vec![Change::write("scripts/stale.ws", b"stale")]));
            assert!(matches!(
                result,
                Err(Error::State(state::Error::AutomationRefused))
            ));
        });
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(w.history(&note.id).unwrap().len(), history);
        assert_eq!(
            w.read_index(|i, _| i.generation().unwrap()),
            index_generation
        );
        assert_eq!(w.definition_changes(), definitions);
        assert_eq!(
            w.current_version(&note.id).unwrap().as_deref(),
            Some(note.version.as_str())
        );
        assert!(!dir.path().join("scripts/stale.ws").exists());
        assert_eq!(
            std::fs::read_dir(dir.path().join("notes/item"))
                .unwrap()
                .count(),
            1
        );
    }
}

#[test]
fn stop_and_resume_during_before_handler_refuses_commit_without_waiting_for_handler() {
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    struct Paused {
        entered: mpsc::Sender<()>,
        proceed: Mutex<mpsc::Receiver<()>>,
    }
    impl writer::Hooks for Paused {
        fn before(&self, _: &mut writer::NoteChange, _: &Author) -> Result<(), String> {
            self.entered.send(()).unwrap();
            self.proceed
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(())
        }
    }
    let (dir, w) = setup();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    let w = Arc::new(w.with_hooks(Box::new(Paused {
        entered: entered_tx,
        proceed: Mutex::new(proceed_rx),
    })));
    let generation = w.state().admit_automation().unwrap();
    let worker = w.clone();
    let thread = std::thread::spawn(move || {
        worker.with_automation(generation, || {
            worker.create(
                "item",
                &Input::Wcl("title = \"Race\"\n".into()),
                &Author::Agent("run".into()),
            )
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    w.state().set_automation_stopped(true).unwrap();
    w.state().set_automation_stopped(false).unwrap();
    proceed_tx.send(()).unwrap();
    assert!(matches!(
        thread.join().unwrap(),
        Err(Error::State(state::Error::AutomationRefused))
    ));
    assert!(!dir.path().join("notes/item").exists());
}

#[test]
fn unused_gate_preserves_existing_behavior_and_new_admissions_work() {
    let (_dir, w) = setup();
    let input = Input::Wcl("title = \"Unchanged\"\n".into());
    let author = Author::Agent("test".into());
    w.create("item", &input, &author).unwrap();
    let generation = w.state().admit_automation().unwrap();
    w.with_automation(generation, || {
        w.create("item", &input, &author).unwrap();
        w.commit_checked::<Error>(|_| Ok(vec![Change::write("scripts/current.ws", b"current")]))
            .unwrap();
    });
    w.state().set_automation_stopped(true).unwrap();
    // Owner/unscoped operations remain available, independent of the author.
    w.create("item", &input, &Author::Session("owner".into()))
        .unwrap();
    w.state().set_automation_stopped(false).unwrap();
    let generation = w.state().admit_automation().unwrap();
    w.with_automation(generation, || w.create("item", &input, &author))
        .unwrap();
}

#[test]
fn stale_asset_type_and_agent_commits_write_nothing() {
    let (dir, w) = setup();
    let generation = w.state().admit_automation().unwrap();
    let index = w.read_index(|i, _| i.generation().unwrap());
    let definitions = w.definition_changes();
    let vault_generation = w.vault().generation().unwrap();
    w.state().set_automation_stopped(true).unwrap();
    w.state().set_automation_stopped(false).unwrap();
    w.with_automation(generation, || {
        assert!(matches!(
            w.put_asset("stale.txt", b"stale".to_vec()),
            Err(Error::State(state::Error::AutomationRefused))
        ));
        assert!(matches!(
            w.put_type(
                "stale",
                "label = \"Stale\"\ntemplate default { tab \"Main\" { slot \"title\" } }\n"
            ),
            Err(Error::State(state::Error::AutomationRefused))
        ));
        assert!(matches!(
            w.put_agent("stale", "label = \"Stale\"\n", |_| Vec::new()),
            Err(Error::State(state::Error::AutomationRefused))
        ));
    });
    assert!(w.assets().unwrap().is_empty());
    assert!(!dir.path().join("types/stale.wcl").exists());
    assert!(!dir.path().join("agents/stale.wcl").exists());
    assert_eq!(w.vault().generation().unwrap(), vault_generation);
    assert_eq!(w.read_index(|i, _| i.generation().unwrap()), index);
    assert_eq!(w.definition_changes(), definitions);
}

#[test]
fn nested_scopes_keep_oldest_generation_and_restore_outer_context() {
    let (dir, w) = setup();
    let old = w.state().admit_automation().unwrap();
    w.state().set_automation_stopped(true).unwrap();
    w.state().set_automation_stopped(false).unwrap();
    let new = w.state().admit_automation().unwrap();
    let write =
        || w.commit_checked::<Error>(|_| Ok(vec![Change::write("scripts/nested.ws", b"ok")]));
    w.with_automation(old, || {
        assert!(matches!(
            w.with_automation(new, write),
            Err(Error::State(state::Error::AutomationRefused))
        ));
        assert!(matches!(
            write(),
            Err(Error::State(state::Error::AutomationRefused))
        ));
    });
    assert!(!dir.path().join("scripts/nested.ws").exists());
    w.with_automation(new, write).unwrap();
}

#[test]
fn stop_waits_for_scoped_commit_then_refuses_next_commit() {
    use std::ops::ControlFlow;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    let (_dir, w) = setup();
    let w = Arc::new(w);
    let generation = w.state().admit_automation().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    let mut first = true;
    w.set_commit_hook(Some(Box::new(move |_| {
        if first {
            first = false;
            entered_tx.send(()).unwrap();
            proceed_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        ControlFlow::Continue(())
    })));
    let worker = w.clone();
    let commit = std::thread::spawn(move || {
        worker.with_automation(generation, || {
            worker.create(
                "item",
                &Input::Wcl("title = \"First\"\n".into()),
                &Author::Agent("run".into()),
            )
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let stopper = w.clone();
    let stop = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        stopper.state().set_automation_stopped(true).unwrap();
        stopped_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let blocked = stopped_rx.recv_timeout(Duration::from_millis(100));
    proceed_tx.send(()).unwrap();
    assert!(matches!(blocked, Err(mpsc::RecvTimeoutError::Timeout)));
    let saved = commit.join().unwrap().unwrap();
    stopped_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    stop.join().unwrap();
    assert_eq!(w.history(&saved.id).unwrap().len(), 1);
    w.set_commit_hook(None);
    assert!(matches!(
        w.with_automation(generation, || w.create(
            "item",
            &Input::Wcl("title = \"Next\"\n".into()),
            &Author::Agent("run".into())
        )),
        Err(Error::State(state::Error::AutomationRefused))
    ));
}
