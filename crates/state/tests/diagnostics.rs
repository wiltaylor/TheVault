use state::diagnostics::state_size;
use state::{Author, State};
use tempfile::TempDir;

#[test]
fn empty_and_missing_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    assert!(state_size(&path, 1).is_err());
    assert!(!path.exists());
    drop(State::open(&path, None).unwrap());
    let before = std::fs::read(&path).unwrap();
    let report = state_size(&path, u64::MAX).unwrap();
    assert!(report.history.is_empty());
    assert_eq!(report.database_file_bytes, before.len() as u64);
    assert_eq!(report.wal_file_bytes, 0);
    assert!(report.warning.is_none());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn mixed_utf8_history_and_live_wal_are_read_only() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let author = Author::Token("fixture".into());
    for (id, kind, text) in [
        ("a", "email", "é"),
        ("a", "email", "abc"),
        ("b", "task", ""),
    ] {
        state
            .record_distinct_version(id, kind, &author, text)
            .unwrap();
    }
    let history = state.history("a").unwrap();
    let database = std::fs::read(&path).unwrap();
    let wal = std::fs::read(path.with_file_name("state.db-wal")).unwrap();
    let report = state_size(&path, 1).unwrap();
    assert_eq!(report.history.len(), 2);
    assert_eq!(report.history[0].note_type, "email");
    assert_eq!(report.history[0].versions, 2);
    assert_eq!(report.history[0].content_bytes, 5);
    assert_eq!(report.history[1].note_type, "task");
    assert_eq!(report.history[1].versions, 1);
    assert_eq!(report.history[1].content_bytes, 0);
    assert_eq!(report.database_file_bytes, database.len() as u64);
    assert_eq!(report.wal_file_bytes, wal.len() as u64);
    assert!(report.warning.unwrap().contains("No history was removed"));
    let total = report.database_file_bytes + report.wal_file_bytes;
    assert!(state_size(&path, total).unwrap().warning.is_some());
    assert!(state_size(&path, total + 1).unwrap().warning.is_none());
    assert_eq!(state.history("a").unwrap(), history);
    assert_eq!(std::fs::read(&path).unwrap(), database);
    assert_eq!(
        std::fs::read(path.with_file_name("state.db-wal")).unwrap(),
        wal
    );
}
