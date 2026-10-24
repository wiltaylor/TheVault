//! What `state.db` records about the shipped defaults written into the vault, and the one-time
//! upgrade steps applied to it.

use state::{ShippedRecord, State};
use tempfile::TempDir;

fn open(dir: &TempDir) -> State {
    State::open(&dir.path().join("state.db"), None).unwrap()
}

#[test]
fn a_shipped_file_records_its_hash_and_its_notice() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert_eq!(state.shipped_file("scripts/device_calls.ws").unwrap(), None);
    state
        .record_shipped("scripts/device_calls.ws", "aa")
        .unwrap();
    assert_eq!(
        state.shipped_file("scripts/device_calls.ws").unwrap(),
        Some(ShippedRecord {
            hash: "aa".into(),
            notified: None,
        })
    );
    state
        .record_shipped_notice("scripts/device_calls.ws", "bb")
        .unwrap();
    state
        .record_shipped("scripts/device_calls.ws", "cc")
        .unwrap();
    drop(state);
    let state = open(&dir);
    assert_eq!(
        state.shipped_file("scripts/device_calls.ws").unwrap(),
        Some(ShippedRecord {
            hash: "cc".into(),
            notified: Some("bb".into()),
        })
    );
}

#[test]
fn a_notice_for_a_file_never_written_is_kept() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    state
        .record_shipped_notice("devices/android.wcl", "bb")
        .unwrap();
    assert_eq!(
        state.shipped_file("devices/android.wcl").unwrap(),
        Some(ShippedRecord {
            hash: String::new(),
            notified: Some("bb".into()),
        })
    );
}

#[test]
fn an_upgrade_step_is_recorded_once() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert!(state.applied_upgrades().unwrap().is_empty());
    state.record_upgrade("0001-first").unwrap();
    state.record_upgrade("0001-first").unwrap();
    state.record_upgrade("0002-second").unwrap();
    drop(state);
    let state = open(&dir);
    assert_eq!(
        state.applied_upgrades().unwrap(),
        ["0001-first", "0002-second"]
    );
}

#[test]
fn a_version_10_state_db_gains_the_shipped_tables() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    drop(open(&dir));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE shipped_files; DROP TABLE upgrades; PRAGMA user_version = 10;",
        )
        .unwrap();
    }
    let state = open(&dir);
    state
        .record_shipped("scripts/device_calls.ws", "aa")
        .unwrap();
    state.record_upgrade("0001-first").unwrap();
    drop(state);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 17);
}
