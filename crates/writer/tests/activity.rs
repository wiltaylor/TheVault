//! What the write path records in the activity log (activity Y2): note changes by their author,
//! coalesced edits, and definition changes by whoever [`writer::acting_as`] names.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use state::{Activity, ActivityFilter, Author, Clock, State};
use store::Vault;
use writer::{Input, Writer, acting_as};

const TASK: &str =
    "label = \"Task\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn open(root: &Path, clock: Arc<TestClock>) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/task.wcl"), TASK).unwrap();
    let vault = Vault::init(root).unwrap();
    let state = State::open_with_clock(&vault.server_dir().join("state.db"), None, clock).unwrap();
    Writer::open(vault, state).unwrap()
}

fn entries(w: &Writer, category: &str) -> Vec<Activity> {
    let mut items = w
        .state()
        .activity(&ActivityFilter {
            category: Some(category.into()),
            ..Default::default()
        })
        .unwrap()
        .items;
    items.reverse();
    items
}

fn kinds(items: &[Activity]) -> Vec<&str> {
    items.iter().map(|a| a.kind.as_str()).collect()
}

#[test]
fn every_note_change_is_recorded_with_its_author() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let w = open(dir.path(), clock.clone());
    let me = Author::Session("owner".into());
    let agent = Author::Agent("helper".into());
    let s = w
        .create("task", &Input::Wcl("title = \"Plan\"\n".into()), &me)
        .unwrap();
    let s = w
        .update(
            &s.id,
            &Input::Wcl("title = \"Plan B\"\n".into()),
            Some(&s.version),
            &me,
        )
        .unwrap();
    clock.0.fetch_add(5_000, Ordering::SeqCst);
    let s = w
        .update(
            &s.id,
            &Input::Wcl("title = \"Plan C\"\n".into()),
            Some(&s.version),
            &me,
        )
        .unwrap();
    let s = w
        .update(
            &s.id,
            &Input::Wcl("title = \"Plan D\"\n".into()),
            Some(&s.version),
            &agent,
        )
        .unwrap();
    clock.0.fetch_add(61_000, Ordering::SeqCst);
    let s = w
        .update(
            &s.id,
            &Input::Wcl("title = \"Plan E\"\n".into()),
            Some(&s.version),
            &agent,
        )
        .unwrap();
    let s = w.set_archived(&s.id, true, Some(&s.version), &me).unwrap();
    let s = w.set_archived(&s.id, false, Some(&s.version), &me).unwrap();
    let gone = w.delete(&s.id, Some(&s.version), &me).unwrap();
    w.restore_from_trash(&s.id, &me).unwrap();
    let s2 = w.read(&s.id).unwrap();
    w.delete(&s.id, Some(&s2.version), &me).unwrap();
    w.purge_from_trash(&s.id).unwrap();

    let got = entries(&w, "notes");
    assert_eq!(
        kinds(&got),
        [
            "note.create",
            "note.update",
            "note.update",
            "note.update",
            "note.archive",
            "note.unarchive",
            "note.delete",
            "note.restore",
            "note.delete",
            "note.purge",
        ]
    );
    assert_eq!(got[0].summary, "Created task \"Plan\"");
    assert_eq!(
        (got[0].actor_kind.as_str(), got[0].actor.as_str()),
        ("session", "owner")
    );
    assert_eq!(got[0].subject_kind.as_deref(), Some("note"));
    assert_eq!(got[0].subject.as_deref(), Some(s.id.as_str()));
    assert_eq!(got[0].detail["type"], "task");
    // The owner's two edits coalesce, the agent's are its own, and a minute later a new entry.
    assert_eq!(got[1].summary, "Edited task \"Plan C\"");
    assert_eq!(got[1].detail["edits"], 2);
    assert_eq!(
        (got[2].actor_kind.as_str(), got[2].actor.as_str()),
        ("agent", "helper")
    );
    assert_eq!(got[2].summary, "Edited task \"Plan D\"");
    assert_eq!(got[3].summary, "Edited task \"Plan E\"");
    assert_eq!(got[6].summary, "Deleted task \"Plan E\" to the trash");
    assert_eq!(got[6].detail["version"], gone.version.as_str());
    assert_eq!(got[9].summary, "Purged task \"Plan E\" for good");
}

#[test]
fn definition_changes_are_recorded_by_the_actor() {
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), Arc::new(TestClock(AtomicI64::new(1_000))));
    {
        let _a = acting_as(&Author::Session("owner".into()));
        w.put_type("memo", &TASK.replace("Task", "Memo")).unwrap();
        w.put_type("memo", &TASK.replace("Task", "Memo note"))
            .unwrap();
    }
    w.put_script("hello", "fn main() {}\n").unwrap();
    w.delete_script("hello").unwrap();
    {
        let _a = acting_as(&Author::Agent("helper".into()));
        w.delete_type("memo").unwrap();
    }
    let got = entries(&w, "definitions");
    let tail: Vec<_> = got
        .iter()
        .rev()
        .take(5)
        .rev()
        .map(|a| (a.kind.as_str(), a.actor_kind.as_str(), a.summary.as_str()))
        .collect();
    assert_eq!(
        tail,
        [
            ("type.create", "session", "Created type memo"),
            ("type.update", "session", "Changed type memo"),
            ("script.create", "server", "Created script hello"),
            ("script.delete", "server", "Deleted script hello"),
            ("type.delete", "agent", "Deleted type memo"),
        ]
    );
    let last = got.last().unwrap();
    assert_eq!(last.subject_kind.as_deref(), Some("type"));
    assert_eq!(last.subject.as_deref(), Some("memo"));
}
