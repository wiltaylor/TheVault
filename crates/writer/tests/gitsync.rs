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

struct CalendarClock;

fn calendar_types(w: &Writer) {
    w.put_type("person", include_str!("../defaults/person.wcl"))
        .unwrap();
    w.put_type("event", include_str!("../defaults/event.wcl"))
        .unwrap();
    w.put_type("event_series", include_str!("../defaults/event_series.wcl"))
        .unwrap();
}

#[test]
fn device_calendar_owner_edits_keep_existing_send_updates_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), Recorder::default()).with_clock(Arc::new(CalendarClock));
    calendar_types(&w);
    let (device, _) = w
        .state()
        .pair_device(&state::NewDevice {
            name: "test",
            platform: "linux",
            capabilities: "{}",
            settings: "{}",
        })
        .unwrap();
    let author = Author::Token(device.token_id);
    let person = w
        .create("person", &Input::Wcl("title = \"Guest\"\n".into()), &user())
        .unwrap();
    let base = "title = \"Invitation\"\ngoogle_ref = \"synthetic\"\n";
    let saved = w
        .create("event", &Input::Wcl(base.into()), &author)
        .unwrap();
    let mut saved = saved;
    let mut text = base.to_owned();
    for edit in [
        "my_response = \"accepted\"\n".to_owned(),
        format!("attendees = [\"{}\"]\n", person.id),
        "status = \"tentative\"\n".to_owned(),
    ] {
        text.push_str(&edit);
        saved = w
            .update(
                &saved.id,
                &Input::Wcl(text.clone()),
                Some(&saved.version),
                &author,
            )
            .unwrap();
    }
    assert!(matches!(
        w.update(
            &saved.id,
            &Input::Wcl(format!("{text}send_updates = true\n")),
            Some(&saved.version),
            &author
        ),
        Err(Error::Rejected(_))
    ));
    assert_eq!(w.read(&saved.id).unwrap().wcl, saved.wcl);
}

impl state::Clock for CalendarClock {
    fn now_ms(&self) -> i64 {
        1_772_352_000_000
    }
}

#[test]
fn calendar_peer_guard_rejects_whole_writes_and_trusts_only_home_feed() {
    for ty in ["event", "event_series"] {
        for forged in [
            "google_ref = \"forged\"\n",
            "etag = \"forged\"\n",
            "attendees = []\n",
            "attendees_raw = [\"forged\"]\n",
            "my_response = \"accepted\"\n",
            "status = \"cancelled\"\n",
            "send_updates = true\n",
            "send_updates = \"all\"\n",
            "dict snapshot { title = \"forged\" }\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let hooks = Recorder::default();
            let w = open(dir.path(), hooks.clone()).with_clock(Arc::new(CalendarClock));
            calendar_types(&w);
            let recurrence = if ty == "event_series" {
                "rule = [\"RRULE:FREQ=DAILY;COUNT=1\"]\nstarts = { start: \"2026-03-10\", end: \"2026-03-11\" }\n"
            } else {
                ""
            };
            let text = format!("title = \"peer\"\n{recurrence}{forged}");
            assert!(matches!(
                w.create_remote(ID, ty, &text, &gitsync()),
                Err(Error::Rejected(_))
            ));
            assert!(w.state().history(ID).unwrap().is_empty());
            assert_eq!(w.note_state(ID).unwrap(), NoteState::Absent);
            assert!(!dir.path().join(format!("notes/{ty}/{ID}.wcl")).exists());

            let base = w.create_remote(ID, "task", FROM_A, &gitsync()).unwrap();
            assert!(matches!(
                w.replace_remote(ID, ty, &text, &base.version, &gitsync()),
                Err(Error::Rejected(_))
            ));
            assert_eq!(w.read(ID).unwrap().wcl, base.wcl);
            assert_eq!(w.state().history(ID).unwrap().len(), 1);
            assert_eq!(w.list(Some(ty)).unwrap().len(), 0);

            // Authenticated home-server feed may carry exactly the same fields.
            w.apply_remote(&writer::Remote::Put {
                id: ID.into(),
                type_name: ty.into(),
                wcl: text.clone(),
            })
            .unwrap();
            let before = w.read(ID).unwrap();
            let changed = text
                .replace("forged", "changed")
                .replace("true", "\"none\"")
                .replace("\"all\"", "\"none\"")
                .replace("cancelled", "confirmed")
                .replace("accepted", "declined");
            assert!(matches!(
                w.replace_remote(ID, ty, &changed, &before.version, &gitsync()),
                Err(Error::Rejected(_))
            ));
            assert_eq!(w.read(ID).unwrap().wcl, before.wcl);
            assert_eq!(w.state().history(ID).unwrap().len(), 2);
            let same = text.replace("\"peer\"", "\"title edit\"");
            w.replace_remote(ID, ty, &same, &before.version, &gitsync())
                .unwrap();
            assert!(hooks.0.lock().unwrap().is_empty());
            let current = w.read(ID).unwrap();
            w.delete(ID, Some(&current.version), &user()).unwrap();
            let history = w.state().history(ID).unwrap().len();
            assert!(matches!(
                w.create_remote(ID, ty, &text, &gitsync()),
                Err(Error::IdTaken(_))
            ));
            assert_eq!(w.state().history(ID).unwrap().len(), history);
        }
    }
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
