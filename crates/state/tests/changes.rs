//! The change feed and device base versions in `state.db` (D20, D29).

use rusqlite::Connection;
use state::{Author, ChangeKey, Error, NewDevice, PrunePolicy, State};
use tempfile::TempDir;

const NOTE_A: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const NOTE_B: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

fn open(dir: &TempDir) -> State {
    State::open(&dir.path().join("state.db"), None).unwrap()
}

fn session() -> Author {
    Author::Session("owner".into())
}

fn keys(state: &State, since: i64) -> Vec<(ChangeKey, bool)> {
    state
        .changes_since(since, 100)
        .unwrap()
        .changes
        .into_iter()
        .map(|c| (c.key, c.present))
        .collect()
}

fn phone() -> NewDevice<'static> {
    NewDevice {
        name: "Pixel",
        platform: "android",
        capabilities: "[]",
        settings: "{}",
    }
}

#[test]
fn each_key_appears_once_at_its_latest_change() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert_eq!(state.changes_since(0, 10).unwrap().latest, 0);

    state
        .record_version(NOTE_A, "task", &session(), "a1")
        .unwrap();
    state
        .record_version(NOTE_B, "task", &session(), "b1")
        .unwrap();
    // A merged browser save still moves the note to the end of the feed.
    state
        .record_version(NOTE_A, "task", &session(), "a2")
        .unwrap();
    assert_eq!(state.history(NOTE_A).unwrap().len(), 1);
    state
        .journal_files(&[("types/task.wcl".into(), true)])
        .unwrap();
    state.trash_note(NOTE_B, &session()).unwrap();

    assert_eq!(
        keys(&state, 0),
        vec![
            (ChangeKey::Note(NOTE_A.into()), true),
            (ChangeKey::File("types/task.wcl".into()), true),
            (ChangeKey::Note(NOTE_B.into()), false),
        ]
    );
    let page = state.changes_since(0, 2).unwrap();
    assert_eq!(page.changes.len(), 2);
    assert_eq!(page.next, page.changes[1].seq);
    // `remaining` counts the entries after the page, not the seqs: note A's first change left a
    // gap.
    assert_eq!(page.remaining, 1);
    let rest = state.changes_since(page.next, 2).unwrap();
    assert_eq!(rest.changes.len(), 1);
    assert_eq!(rest.next, rest.latest);
    assert_eq!(rest.remaining, 0);
    assert_eq!(state.changes_since(0, 10).unwrap().remaining, 0);
    let empty = state.changes_since(rest.next, 2).unwrap();
    assert!(empty.changes.is_empty());
    assert_eq!(empty.next, rest.next);

    // Back from the trash, the note is live again and last.
    state.restore_from_trash(NOTE_B).unwrap();
    state
        .record_version(NOTE_B, "task", &session(), "b1")
        .unwrap();
    assert_eq!(
        keys(&state, rest.next),
        vec![(ChangeKey::Note(NOTE_B.into()), true)]
    );
}

#[test]
fn reconciling_files_journals_only_what_differs_from_disk() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let on_disk = vec!["types/task.wcl".to_owned(), "scripts/a.ws".to_owned()];
    assert_eq!(state.reconcile_files(&on_disk).unwrap(), 2);
    let latest = state.changes_since(0, 10).unwrap().latest;
    assert_eq!(state.reconcile_files(&on_disk).unwrap(), 0);
    assert_eq!(state.changes_since(0, 10).unwrap().latest, latest);

    // A file gone from disk is journaled as deleted, once.
    assert_eq!(state.reconcile_files(&on_disk[..1]).unwrap(), 1);
    assert_eq!(
        keys(&state, latest),
        vec![(ChangeKey::File("scripts/a.ws".into()), false)]
    );
    assert_eq!(state.reconcile_files(&on_disk[..1]).unwrap(), 0);
}

#[test]
fn bases_must_exist_and_go_with_their_device() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let v1 = state
        .record_version(NOTE_A, "task", &Author::Token("t".into()), "a1")
        .unwrap();
    state
        .record_version(NOTE_A, "task", &Author::Token("t".into()), "a2")
        .unwrap();
    let (device, _) = state.pair_device(&phone()).unwrap();

    assert!(matches!(
        state.set_device_bases("nope", &[]),
        Err(Error::UnknownDevice(_))
    ));
    assert!(matches!(
        state.set_device_bases(&device.id, &[(NOTE_B.into(), v1.token.clone())]),
        Err(Error::UnknownVersion { .. })
    ));
    state
        .set_device_bases(&device.id, &[(NOTE_A.into(), v1.token.clone())])
        .unwrap();
    assert_eq!(
        state.device_bases(&device.id).unwrap(),
        vec![(NOTE_A.to_owned(), v1.token.clone())]
    );
    let keep_one = PrunePolicy {
        keep_versions: Some(1),
        max_age_ms: None,
    };
    assert_eq!(state.prune(&keep_one).unwrap(), 0);

    state.revoke_device(&device.id).unwrap();
    assert_eq!(state.prune(&keep_one).unwrap(), 1);
    assert!(state.version(NOTE_A, &v1.token).unwrap().is_none());
}

#[test]
fn a_version_8_database_puts_its_notes_in_the_feed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    {
        let state = State::open(&path, None).unwrap();
        state
            .record_version(NOTE_A, "task", &session(), "a1")
            .unwrap();
        state
            .record_version(NOTE_B, "task", &session(), "b1")
            .unwrap();
        state.trash_note(NOTE_B, &session()).unwrap();
    }
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE changes; DROP TABLE device_bases; PRAGMA user_version = 8;")
            .unwrap();
    }
    let state = State::open(&path, None).unwrap();
    assert_eq!(
        keys(&state, 0),
        vec![
            (ChangeKey::Note(NOTE_A.into()), true),
            (ChangeKey::Note(NOTE_B.into()), false),
        ]
    );
}
