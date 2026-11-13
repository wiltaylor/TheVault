use state::State;

#[test]
fn transitions_restart_and_stale_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    assert_eq!(state.automation_state().unwrap().generation, 0);
    let old = state.admit_automation().unwrap();
    drop(state.automation_commit(old).unwrap());
    assert_eq!(state.set_automation_stopped(true).unwrap().generation, 1);
    assert!(state.admit_automation().is_err());
    assert!(state.automation_commit(old).is_err());
    drop(state);
    let state = State::open(&path, None).unwrap();
    assert!(state.automation_state().unwrap().stopped);
    assert!(state.admit_automation().is_err());
    assert_eq!(state.set_automation_stopped(false).unwrap().generation, 2);
    assert!(state.automation_commit(old).is_err());
    let new = state.admit_automation().unwrap();
    drop(state.automation_commit(new).unwrap());
    assert_eq!(state.set_automation_stopped(false).unwrap().generation, 3);
    assert!(state.automation_commit(new).is_err());
}

#[test]
fn migration_from_28_and_29_preserves_rows() {
    for version in [28, 29] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let state = State::open(&path, None).unwrap();
        state.set_meta("existing", "preserved").unwrap();
        let (_, secret) = state.create_token("legacy", state::Scope::Read).unwrap();
        drop(state);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE automation_control;")
            .unwrap();
        if version == 28 {
            conn.execute_batch("ALTER TABLE tokens DROP COLUMN expires_ms;")
                .unwrap();
        }
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);
        let state = State::open(&path, None).unwrap();
        assert_eq!(
            state.meta("existing").unwrap().as_deref(),
            Some("preserved")
        );
        assert!(state.authenticate_token(&secret).unwrap().is_some());
        assert!(!state.automation_state().unwrap().stopped);
        assert_eq!(state.automation_state().unwrap().generation, 0);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, crate::EXPECTED_SCHEMA_VERSION);
    }
}

#[test]
fn unreadable_state_refuses_admission_transition_and_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let generation = state.admit_automation().unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE automation_control;")
        .unwrap();
    assert!(state.automation_state().is_err());
    assert!(state.admit_automation().is_err());
    assert!(state.set_automation_stopped(false).is_err());
    assert!(state.automation_commit(generation).is_err());
}

#[test]
fn poisoned_commit_gate_still_allows_durable_stop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let generation = state.admit_automation().unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = state.automation_commit(generation).unwrap();
        panic!("injected commit panic");
    }));
    assert!(panic.is_err());
    state.set_automation_stopped(true).unwrap();
    assert!(state.admit_automation().is_err());
    assert!(state.automation_commit(generation).is_err());
    drop(state);
    assert!(State::open(&path, None).unwrap().automation_state().unwrap().stopped);
}
