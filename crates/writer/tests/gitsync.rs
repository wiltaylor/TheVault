//! The writes git sync makes for notes it imports from another instance (git-sync D7, D15, D17,
//! D28): another vault's text written under its own id and an author naming the instance, with
//! no handler run, a base version guarding a user's edit, definitions put only when missing, and
//! a deleted note told from one never seen.

use std::path::Path;
use std::sync::{Arc, Mutex};

use schema::Schema;
use state::{Author, State};
use store::Vault;
use writer::{Error, Event, Hooks, Input, NoteChange, NoteState, Saved, Writer};

const TASK: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate"
  }
}
"#;

const ID: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";

/// The note as instance A wrote it, stamps and all.
const FROM_A: &str = r#"title    = "From A"
created  = "2026-10-12T09:15:00Z"
updated  = "2026-10-12T10:42:07Z"
estimate = 3
"#;

const FROM_A_LATER: &str = r#"title    = "From A, edited"
created  = "2026-10-12T09:15:00Z"
updated  = "2026-10-13T08:00:00Z"
estimate = 5
"#;

/// Records every handler the write path runs.
#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Hooks for Recorder {
    fn before(&self, change: &mut NoteChange, _: &Author) -> Result<(), String> {
        self.0.lock().unwrap().push(format!("before {}", change.id));
        Ok(())
    }

    fn after(&self, saved: &[Saved], _: &Author, _: &Schema) {
        let mut seen = self.0.lock().unwrap();
        for s in saved {
            seen.push(format!("after {}", s.id));
        }
    }
}

fn open(root: &Path, hooks: Recorder) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/task.wcl"), TASK).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state)
        .unwrap()
        .with_hooks(Box::new(hooks))
}

fn gitsync() -> Author {
    Author::Task("gitsync A".into())
}

fn user() -> Author {
    Author::Session("owner".into())
}

#[test]
fn an_imported_note_keeps_its_id_and_text_names_its_instance_and_runs_no_handler() {
    let dir = tempfile::tempdir().unwrap();
    let hooks = Recorder::default();
    let w = open(dir.path(), hooks.clone());

    let saved = w.create_remote(ID, "task", FROM_A, &gitsync()).unwrap();
    assert_eq!(saved.id, ID);
    assert_eq!(saved.event, Event::Create);
    assert_eq!(saved.wcl, FROM_A);
    assert_eq!(w.read(ID).unwrap().wcl, FROM_A);
    let replaced = w
        .replace_remote(ID, "task", FROM_A_LATER, &saved.version, &gitsync())
        .unwrap();
    assert_eq!(replaced.event, Event::Update);

    let history = w.history(ID).unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|v| v.author == gitsync()), "{history:?}");
    assert_eq!(history[1].content, FROM_A_LATER);
    assert_eq!(hooks.0.lock().unwrap().len(), 0, "no handler runs");

    // The index holds the imported note like any other.
    let listed: Vec<_> = w
        .list(Some("task"))
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert_eq!(listed, ["From A, edited"]);
}

#[test]
fn an_import_does_not_overwrite_a_note_the_user_edited_since_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), Recorder::default());
    let imported = w.create_remote(ID, "task", FROM_A, &gitsync()).unwrap();
    let edited = w
        .update(
            ID,
            &Input::Wcl("title = \"Edited here\"\n".into()),
            Some(&imported.version),
            &user(),
        )
        .unwrap();

    let err = w
        .replace_remote(ID, "task", FROM_A_LATER, &imported.version, &gitsync())
        .unwrap_err();
    match err {
        Error::Conflict {
            current_version,
            current,
            ..
        } => {
            assert_eq!(current_version, edited.version);
            assert_eq!(current, edited.wcl);
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(w.read(ID).unwrap().wcl, edited.wcl);

    // Written over the version the user left, it goes through.
    let merged = w
        .replace_remote(ID, "task", FROM_A_LATER, &edited.version, &gitsync())
        .unwrap();
    assert_eq!(w.read(ID).unwrap().wcl, FROM_A_LATER);
    // The same text again records nothing.
    let again = w
        .replace_remote(ID, "task", FROM_A_LATER, &merged.version, &gitsync())
        .unwrap();
    assert_eq!(again.version, merged.version);
    assert_eq!(w.history(ID).unwrap().len(), 3);
}

#[test]
fn creating_under_an_id_already_used_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), Recorder::default());
    let saved = w.create_remote(ID, "task", FROM_A, &gitsync()).unwrap();
    assert!(matches!(
        w.create_remote(ID, "task", FROM_A_LATER, &gitsync()),
        Err(Error::IdTaken(_))
    ));
    w.delete(ID, Some(&saved.version), &user()).unwrap();
    assert!(matches!(
        w.create_remote(ID, "task", FROM_A, &gitsync()),
        Err(Error::IdTaken(_))
    ));
    assert!(matches!(
        w.replace_remote(ID, "task", FROM_A, &saved.version, &gitsync()),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        w.create_remote("01JB8Z4K2MQ7V3X9T5R6W8Y2ZB", "missing", FROM_A, &gitsync()),
        Err(Error::UnknownType(_))
    ));
}

#[test]
fn a_note_is_live_trashed_or_absent() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), Recorder::default());
    assert_eq!(w.note_state(ID).unwrap(), NoteState::Absent);
    let saved = w.create_remote(ID, "task", FROM_A, &gitsync()).unwrap();
    assert_eq!(
        w.note_state(ID).unwrap(),
        NoteState::Live {
            type_name: "task".into(),
            version: saved.version.clone(),
        }
    );
    w.delete(ID, Some(&saved.version), &user()).unwrap();
    assert_eq!(w.note_state(ID).unwrap(), NoteState::Trashed);
    w.restore_from_trash(ID, &user()).unwrap();
    assert!(matches!(w.note_state(ID).unwrap(), NoteState::Live { .. }));
    let version = w.current_version(ID).unwrap().unwrap();
    w.delete(ID, Some(&version), &user()).unwrap();
    w.purge_from_trash(ID).unwrap();
    assert_eq!(w.note_state(ID).unwrap(), NoteState::Absent);
}

const STATUS: &str = "label = \"Status\"\n\nterm \"Open\"\n";
const OTHER_STATUS: &str = "label = \"Other\"\n\nterm \"Closed\"\n";
const NOTE_TYPE: &str = r#"label = "Imported"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

#[test]
fn a_definition_is_put_only_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), Recorder::default());

    assert!(
        w.put_taxonomy_if_missing("status", STATUS, &gitsync())
            .unwrap()
    );
    assert!(
        !w.put_taxonomy_if_missing("status", OTHER_STATUS, &gitsync())
            .unwrap()
    );
    assert_eq!(w.read_taxonomy("status").unwrap(), STATUS);

    assert!(!w.put_type_if_missing("task", NOTE_TYPE).unwrap());
    assert_eq!(w.read_type("task").unwrap(), TASK);
    assert!(w.put_type_if_missing("imported", NOTE_TYPE).unwrap());
    assert!(!w.put_type_if_missing("imported", TASK).unwrap());
    assert_eq!(w.read_type("imported").unwrap(), NOTE_TYPE);

    // An imported note of the new type can be written straight away.
    w.create_remote(ID, "imported", "title = \"New type\"\n", &gitsync())
        .unwrap();
}
