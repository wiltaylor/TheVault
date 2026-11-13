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
