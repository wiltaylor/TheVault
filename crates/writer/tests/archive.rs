//! Archiving: `archived = true` in the note file, written only by [`Writer::set_archived`] as a
//! versioned update, kept by every other write, and mirrored in the index so listings, queries
//! and searches leave archived notes out unless asked.

use std::path::Path;
use std::sync::{Arc, Mutex};

use schema::Schema;
use state::{Author, State};
use store::Vault;
use writer::{Archived, Error, Event, Hooks, Input, Saved, Writer};

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

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn setup(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/task.wcl"), TASK).unwrap();
    open(root)
}

fn author() -> Author {
    Author::Session("test".into())
}

#[test]
fn oversized_history_creates_updates_recovers_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let w = setup(dir.path());
    let by = Author::Token("large-fixture".into());
    let input = Input::Wcl(format!(
        "title = \"{}\"\n",
        "x".repeat(64 * 1024 * 1024 + 1)
    ));
    let a = w.create("task", &input, &by).unwrap();
    assert!(a.wcl.len() > 64 * 1024 * 1024);
    let b = w
        .update(
            &a.id,
            &Input::Wcl("title = \"small\"\n".into()),
            Some(&a.version),
            &by,
        )
        .unwrap();
    // Fail after the file commit so reopen must record the oversized snapshot.
    w.set_commit_hook(Some(Box::new(|step| {
        if step == store::CommitStep::Applied(0) {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    })));
    assert!(w.update(&a.id, &input, Some(&b.version), &by).is_err());
    drop(w);
    let w = open(dir.path());
    let current = w.current_version(&a.id).unwrap().unwrap();
    assert_ne!(current, b.version);
    let path = dir.path().join(format!("notes/task/{}.wcl", a.id));
    assert!(w.version(&a.id, &current).unwrap().content == std::fs::read_to_string(path).unwrap());
    let restored = w
        .restore_version(&a.id, &a.version, Some(&current), &by)
        .unwrap();
    // Restore is a new write: only its updated stamp may differ from the snapshot.
    assert!(
        restored
            .wcl
            .lines()
            .filter(|line| !line.starts_with("updated "))
            .eq(a.wcl.lines().filter(|line| !line.starts_with("updated ")))
    );
    assert!(w.version(&a.id, &a.version).unwrap().content == a.wcl);
    let conn = rusqlite::Connection::open(dir.path().join(".vault/state.db")).unwrap();
    let kind: String = conn
        .query_row(
            "SELECT typeof(content) FROM versions WHERE token=?1",
            [&a.version],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "text");
}

#[test]
fn compressed_and_legacy_restore_are_exact_and_corruption_writes_nothing() {
    struct FixedClock;

    impl state::Clock for FixedClock {
        fn now_ms(&self) -> i64 {
            1_700_000_000_000
        }
    }

    let dir = tempfile::tempdir().unwrap();
    // Restore writes a fresh updated stamp. Freeze time to compare every byte.
    let w = setup(dir.path()).with_clock(Arc::new(FixedClock));
    let by = Author::Token("fixture".into());
    let a = w
        .create(
            "task",
            &Input::Wcl(format!("title = \"{}\"\n", "雪".repeat(10000))),
            &by,
        )
        .unwrap();
    let b = w
        .update(
            &a.id,
            &Input::Wcl("title = \"changed\"\n".into()),
            Some(&a.version),
            &by,
        )
        .unwrap();
    let restored = w
        .restore_version(&a.id, &a.version, Some(&b.version), &by)
        .unwrap();
    assert_eq!(restored.wcl.as_bytes(), a.wcl.as_bytes());
    let conn = rusqlite::Connection::open(dir.path().join(".vault/state.db")).unwrap();
    conn.execute(
        "UPDATE versions SET content=?1, content_bytes=NULL WHERE token=?2",
        rusqlite::params![b.wcl, b.version],
    )
    .unwrap();
    let old = w
        .restore_version(&a.id, &b.version, Some(&restored.version), &by)
        .unwrap();
    assert_eq!(old.wcl.as_bytes(), b.wcl.as_bytes());
    let path = dir.path().join(format!("notes/task/{}.wcl", a.id));
    let before = std::fs::read(&path).unwrap();
    let mut bomb: Vec<u8> = conn
        .query_row(
            "SELECT content FROM versions WHERE token=?1",
            [&a.version],
            |r| r.get(0),
        )
        .unwrap();
    bomb[4..12].copy_from_slice(&(64 * 1024 * 1024 + 1u64).to_le_bytes());
    conn.execute(
        "UPDATE versions SET content=?1 WHERE token=?2",
        rusqlite::params![bomb, a.version],
    )
    .unwrap();
    let error = w
        .restore_version(&a.id, &a.version, Some(&old.version), &by)
        .unwrap_err();
    assert!(error.to_string().contains("limit"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    conn.execute(
        "UPDATE versions SET content=x'56485302' WHERE token=?1",
        [&a.version],
    )
    .unwrap();
    assert!(
        w.restore_version(&a.id, &a.version, Some(&old.version), &by)
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

fn create(w: &Writer, title: &str) -> Saved {
    w.create(
        "task",
        &Input::Wcl(format!("title = \"{title}\"\n")),
        &author(),
    )
    .unwrap()
}

fn archive(w: &Writer, s: &Saved) -> Saved {
    w.set_archived(&s.id, true, Some(&s.version), &author())
        .unwrap()
}

fn listed(w: &Writer, archived: Archived) -> Vec<String> {
    w.list_with(Some("task"), archived)
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect()
}

fn queried(w: &Writer, text: &str) -> Vec<String> {
    w.read_index(|index, schema| {
        let q = query::parse(text).unwrap();
        let ctx = query::Context::new(index, schema);
        query::run(&q, &ctx, &query::PageRequest::default())
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.title)
            .collect()
    })
}

#[test]
fn archiving_is_a_versioned_write_that_puts_the_flag_after_the_stamps() {
    let dir = tempfile::tempdir().unwrap();
    let w = setup(dir.path());
    let a = w
        .create(
            "task",
            &Input::Wcl("title = \"Old plan\"\nestimate = 3\n".into()),
            &author(),
        )
        .unwrap();

    // The base version is required and must be current, like any other write.
    assert!(matches!(
        w.set_archived(&a.id, true, None, &author()),
        Err(Error::BaseRequired(_))
    ));
    assert!(matches!(
        w.set_archived(&a.id, true, Some("stale"), &author()),
        Err(Error::Conflict { .. })
    ));

    let archived = archive(&w, &a);
    assert_eq!(archived.event, Event::Update);
    assert_ne!(archived.version, a.version);
    let lines: Vec<&str> = archived.wcl.lines().map(str::trim_start).collect();
    let at = |prefix: &str| lines.iter().position(|l| l.starts_with(prefix)).unwrap();
    assert!(at("updated") < at("archived") && at("archived") < at("estimate"));
    assert!(archived.wcl.contains("archived = true"), "{}", archived.wcl);
    assert!(writer::is_archived(&w.read(&a.id).unwrap().doc));

    let history = w.history(&a.id).unwrap();
    assert_eq!(history.len(), 2, "archiving records a version");
    assert!(history[1].content.contains("archived = true"));

    // Archiving an archived note changes nothing.
    let again = archive(&w, &archived);
    assert_eq!(again.version, archived.version);
    assert_eq!(w.history(&a.id).unwrap().len(), 2);

    let back = w
        .set_archived(&a.id, false, Some(&archived.version), &author())
        .unwrap();
    assert!(!back.wcl.contains("archived"), "{}", back.wcl);
    assert!(!writer::is_archived(&w.read(&a.id).unwrap().doc));
    assert_eq!(w.history(&a.id).unwrap().len(), 3);
}

#[test]
fn a_write_keeps_the_archived_state_whatever_it_sends() {
    let dir = tempfile::tempdir().unwrap();
    let w = setup(dir.path());
    let a = archive(&w, &create(&w, "Kept"));

    // A client that does not know about archiving leaves the flag out: the note stays archived.
    let edited = w
        .update(
            &a.id,
            &Input::Wcl("title = \"Kept, edited\"\n".into()),
            Some(&a.version),
            &author(),
        )
        .unwrap();
    assert!(edited.wcl.contains("archived = true"), "{}", edited.wcl);

    // Nor can a write archive or unarchive a note by sending the flag.
    let sent = w
        .update(
            &a.id,
            &Input::Wcl("title = \"Kept\"\narchived = false\n".into()),
            Some(&edited.version),
            &author(),
        )
        .unwrap();
    assert!(sent.wcl.contains("archived = true"), "{}", sent.wcl);
    let b = w
        .create(
            "task",
            &Input::Wcl("title = \"New\"\narchived = true\n".into()),
            &author(),
        )
        .unwrap();
    assert!(!b.wcl.contains("archived"), "{}", b.wcl);

    // Restoring an earlier version keeps the state the note has now.
    let first = &w.history(&a.id).unwrap()[0];
    let restored = w
        .restore_version(&a.id, &first.token, Some(&sent.version), &author())
        .unwrap();
    assert!(restored.wcl.contains("archived = true"), "{}", restored.wcl);

    // A note deleted while archived comes back from the trash archived.
    w.delete(&a.id, Some(&restored.version), &author()).unwrap();
    let back = w.restore_from_trash(&a.id, &author()).unwrap();
    assert!(back.wcl.contains("archived = true"), "{}", back.wcl);
}

#[test]
fn listings_and_queries_leave_archived_notes_out_unless_asked() {
    let dir = tempfile::tempdir().unwrap();
    let w = setup(dir.path());
    let a = create(&w, "Archived task");
    create(&w, "Live task");
    archive(&w, &a);

    assert_eq!(listed(&w, Archived::Exclude), ["Live task"]);
    assert_eq!(listed(&w, Archived::Only), ["Archived task"]);
    assert_eq!(
        listed(&w, Archived::Include),
        ["Archived task", "Live task"]
    );
    assert_eq!(w.list(None).unwrap().len(), 1);

    assert_eq!(queried(&w, "from task sort title"), ["Live task"]);
    assert_eq!(queried(&w, "from * sort title"), ["Live task"]);
    assert_eq!(queried(&w, "from task where archived"), ["Archived task"]);
    assert_eq!(queried(&w, "from task where !archived"), ["Live task"]);
    assert_eq!(
        queried(&w, "from task include archived sort title"),
        ["Archived task", "Live task"]
    );
    assert_eq!(
        queried(
            &w,
            "from task where archived || title == \"Live task\" sort title"
        ),
        ["Archived task", "Live task"]
    );
    assert_eq!(queried(&w, "from task search \"task\""), ["Live task"]);
    assert_eq!(
        queried(&w, "from task include archived search \"archived\""),
        ["Archived task"]
    );
    assert_eq!(
        queried(&w, "from task where archived search \"task\""),
        ["Archived task"]
    );
}

#[test]
fn the_index_flag_survives_a_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let w = setup(dir.path());
    let a = create(&w, "Shelved");
    create(&w, "Current");
    archive(&w, &a);
    let before = w.read_index(|index, _| index.dump().unwrap());
    assert!(
        before.contains("\t0\t1\t\"{"),
        "the archived column holds 1:\n{before}"
    );
    drop(w);

    // A missing index is rebuilt from the files on open, and holds the same rows.
    std::fs::remove_file(dir.path().join(".vault/index.db")).unwrap();
    let w = open(dir.path());
    assert_eq!(w.read_index(|index, _| index.dump().unwrap()), before);
    assert_eq!(listed(&w, Archived::Only), ["Shelved"]);
    assert_eq!(
        queried(&w, "from task search \"shelved\""),
        Vec::<String>::new()
    );
}

/// Records the events the write path runs handlers for.
struct Recorder(Arc<Mutex<Vec<(String, Event, bool)>>>);

impl Hooks for Recorder {
    fn after(&self, saved: &[Saved], _: &Author, _: &Schema) {
        let mut seen = self.0.lock().unwrap();
        for s in saved {
            seen.push((s.id.clone(), s.event, s.wcl.contains("archived = true")));
        }
    }
}

#[test]
fn archiving_runs_the_update_handlers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("types")).unwrap();
    std::fs::write(dir.path().join("types/task.wcl"), TASK).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let w = open(dir.path()).with_hooks(Box::new(Recorder(seen.clone())));
    let a = create(&w, "Watched");
    let archived = archive(&w, &a);
    w.set_archived(&a.id, false, Some(&archived.version), &author())
        .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(
        *seen,
        [
            (a.id.clone(), Event::Create, false),
            (a.id.clone(), Event::Update, true),
            (a.id.clone(), Event::Update, false),
        ]
    );
}
