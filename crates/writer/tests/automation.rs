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
fn run_context_bridges_admission_at_note_and_definition_commits() {
    let (dir, w) = setup();
    let context = script_core::RunContext::default();
    let old = w.state().admit_automation().unwrap();
    context.capture_stop_generation(old);
    context.restrict();
    // Restriction alone adds no refusal, and a live admission still commits.
    context.scope(|| w.put_script("live", "fn run() {}\n")).unwrap();
    w.state().set_automation_stopped(true).unwrap();
    w.state().set_automation_stopped(false).unwrap();
    let fresh = w.state().admit_automation().unwrap();
    let child = script_core::RunContext::default();
    child.capture_stop_generation(fresh);
    let result = context.scope(|| child.scope(|| {
        w.with_automation(fresh, || w.put_script("stale", "fn run() {}\n"))
    }));
    assert!(matches!(result, Err(Error::State(state::Error::AutomationRefused))));
    assert!(!dir.path().join("scripts/stale.ws").exists());
    assert_eq!(child.stop_generation(), Some(old));
    assert!(child.is_restricted());
    let result = child.scope(|| w.create(
        "item", &Input::Wcl("title = \"Old\"\n".into()),
        &Author::Session("test".into()),
    ));
    assert!(matches!(result, Err(Error::State(state::Error::AutomationRefused))));
    // No context or credential is installed after the synchronous closure.
    assert!(script_core::RunContext::current().is_none());
    w.put_script("owner", "fn run() {}\n").unwrap();
}

#[test]
fn empty_run_context_preserves_unscoped_writer_behavior() {
    let (_dir, w) = setup();
    w.state().set_automation_stopped(true).unwrap();
    let context = script_core::RunContext::default();
    context.scope(|| w.put_script("unscoped", "fn run() {}\n")).unwrap();
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
