//! What `state.db` keeps of the notes git sync shares with other instances.

use state::{GitsyncNote, State};
use tempfile::TempDir;

const NOTE_A: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const NOTE_B: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

fn open(dir: &TempDir) -> State {
    State::open(&dir.path().join("state.db"), None).unwrap()
}

fn row(note: &str, peer: &str, base: &[u8]) -> GitsyncNote {
    GitsyncNote {
        note_id: note.into(),
        peer_vault_id: peer.into(),
        origin: "origin".into(),
        base: base.to_vec(),
        base_hash: format!("hash of {}", String::from_utf8_lossy(base)),
        archived_by_sync: false,
    }
}

#[test]
fn a_row_is_stored_replaced_and_kept_across_opens() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert_eq!(state.gitsync_get(NOTE_A, "peer1").unwrap(), None);

    state.gitsync_put(&row(NOTE_A, "peer1", b"one")).unwrap();
    let mut newer = row(NOTE_A, "peer1", b"two\0bytes");
    newer.archived_by_sync = true;
    state.gitsync_put(&newer).unwrap();
    drop(state);

    let state = open(&dir);
    assert_eq!(state.gitsync_get(NOTE_A, "peer1").unwrap(), Some(newer));
    assert_eq!(state.gitsync_get(NOTE_A, "peer2").unwrap(), None);
}

#[test]
fn rows_are_listed_by_peer_and_by_note() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    state.gitsync_put(&row(NOTE_B, "peer1", b"b1")).unwrap();
    state.gitsync_put(&row(NOTE_A, "peer2", b"a2")).unwrap();
    state.gitsync_put(&row(NOTE_A, "peer1", b"a1")).unwrap();

    let by_peer: Vec<_> = state
        .gitsync_rows_for_peer("peer1")
        .unwrap()
        .into_iter()
        .map(|r| (r.note_id, r.base))
        .collect();
    assert_eq!(
        by_peer,
        [
            (NOTE_A.to_owned(), b"a1".to_vec()),
            (NOTE_B.to_owned(), b"b1".to_vec())
        ]
    );
    let by_note: Vec<_> = state
        .gitsync_rows_for_note(NOTE_A)
        .unwrap()
        .into_iter()
        .map(|r| r.peer_vault_id)
        .collect();
    assert_eq!(by_note, ["peer1", "peer2"]);
    assert!(state.gitsync_rows_for_peer("peer3").unwrap().is_empty());
}

#[test]
fn the_imported_rows_are_those_another_instance_created() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let mut own = row(NOTE_B, "peer1", b"b1");
    own.origin = "me".into();
    state.gitsync_put(&own).unwrap();
    state.gitsync_put(&row(NOTE_A, "peer2", b"a2")).unwrap();
    state.gitsync_put(&row(NOTE_A, "peer1", b"a1")).unwrap();

    let imported: Vec<_> = state
        .gitsync_imported("me")
        .unwrap()
        .into_iter()
        .map(|r| (r.note_id, r.peer_vault_id))
        .collect();
    assert_eq!(
        imported,
        [
            (NOTE_A.to_owned(), "peer1".to_owned()),
            (NOTE_A.to_owned(), "peer2".to_owned())
        ]
    );
}

#[test]
fn archived_by_sync_is_set_and_cleared_on_one_row() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    state.gitsync_put(&row(NOTE_A, "peer1", b"a1")).unwrap();
    state.gitsync_put(&row(NOTE_A, "peer2", b"a2")).unwrap();

    assert!(
        state
            .gitsync_set_archived_by_sync(NOTE_A, "peer1", true)
            .unwrap()
    );
    assert!(
        state
            .gitsync_get(NOTE_A, "peer1")
            .unwrap()
            .unwrap()
            .archived_by_sync
    );
    assert!(
        !state
            .gitsync_get(NOTE_A, "peer2")
            .unwrap()
            .unwrap()
            .archived_by_sync
    );

    assert!(
        state
            .gitsync_set_archived_by_sync(NOTE_A, "peer1", false)
            .unwrap()
    );
    assert!(
        !state
            .gitsync_get(NOTE_A, "peer1")
            .unwrap()
            .unwrap()
            .archived_by_sync
    );
    assert!(
        !state
            .gitsync_set_archived_by_sync(NOTE_B, "peer1", true)
            .unwrap()
    );
    assert_eq!(state.gitsync_get(NOTE_B, "peer1").unwrap(), None);
}

#[test]
fn clearing_removes_every_row() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    state.gitsync_put(&row(NOTE_A, "peer1", b"a1")).unwrap();
    state.gitsync_put(&row(NOTE_B, "peer2", b"b2")).unwrap();
    state.gitsync_clear().unwrap();
    assert!(state.gitsync_rows_for_peer("peer1").unwrap().is_empty());
    assert!(state.gitsync_rows_for_note(NOTE_B).unwrap().is_empty());
}

#[test]
fn a_version_17_state_db_gains_the_gitsync_table() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    drop(open(&dir));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE gitsync_notes; PRAGMA user_version = 17;")
            .unwrap();
    }
    let state = open(&dir);
    state.gitsync_put(&row(NOTE_A, "peer1", b"a1")).unwrap();
    assert!(state.gitsync_get(NOTE_A, "peer1").unwrap().is_some());
}
