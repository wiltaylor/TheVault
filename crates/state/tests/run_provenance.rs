use std::sync::Arc;

use state::{Clock, State};

struct FakeClock;
impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        1234
    }
}

fn open(path: &std::path::Path) -> State {
    State::open_with_clock(path, None, Arc::new(FakeClock)).unwrap()
}

#[test]
fn restart_preserves_restriction_and_original_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s.admit_automation().unwrap();
    let restricted = s.create_run_provenance(true, Some(old)).unwrap();
    s.seal_run_provenance(restricted.id()).unwrap();
    let clean = s.create_run_provenance(false, None).unwrap();
    s.seal_run_provenance(clean.id()).unwrap();
    drop(s);
    let s = open(&path);
    let restored = s.load_run_provenance(restricted.id());
    assert!(restored.is_restricted());
    assert_eq!(restored.stop_generation(), Some(old));
    drop(
        s.automation_commit(restored.stop_generation().unwrap())
            .unwrap(),
    );
    let clean = s.load_run_provenance(clean.id());
    assert!(!clean.is_restricted());
    assert_eq!(clean.stop_generation(), None);
    s.set_automation_stopped(true).unwrap();
    s.set_automation_stopped(false).unwrap();
    drop(s);
    let s = open(&path);
    let restored = s.load_run_provenance(restricted.id());
    assert_eq!(restored.stop_generation(), Some(old));
    assert!(
        s.automation_commit(restored.stop_generation().unwrap())
            .is_err()
    );
    drop(s.automation_commit(s.admit_automation().unwrap()).unwrap());
}

#[test]
fn existing_references_keep_late_restrictions_and_oldest_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s.admit_automation().unwrap();
    let a = s.create_run_provenance(false, Some(old)).unwrap();
    let b = s.create_run_provenance(false, None).unwrap();
    let c = s.create_run_provenance(false, None).unwrap();
    s.join_run_provenance(a.id(), b.id()).unwrap();
    s.join_run_provenance(c.id(), b.id()).unwrap();
    s.join_run_provenance(b.id(), a.id()).unwrap(); // no cycle
    s.set_automation_stopped(false).unwrap();
    let newer = s.admit_automation().unwrap();
    s.update_run_provenance(b.id(), true, Some(newer)).unwrap();
    s.update_run_provenance(c.id(), false, None).unwrap();
    s.seal_run_provenance(a.id()).unwrap();
    drop(s);
    let s = open(&path);
    for id in [a.id(), b.id(), c.id()] {
        let p = s.load_run_provenance(id);
        assert_eq!(p.id(), id);
        assert!(p.is_restricted());
        assert_eq!(p.stop_generation(), Some(old));
    }
}

#[test]
fn missing_unreadable_and_unknown_records_are_restricted_and_stale() {
    for damage in [
        "DELETE FROM run_provenance",
        "UPDATE run_provenance SET format=2",
        "UPDATE run_provenance SET restricted=2",
        "UPDATE run_provenance SET restricted='bad'",
        "UPDATE run_provenance SET stop_generation='bad'",
        "UPDATE run_provenance SET stop_generation=-2",
        "UPDATE run_provenance SET stop_generation=99999",
        "UPDATE run_provenance SET parent='missing'",
        "UPDATE run_provenance SET parent=id",
        "DROP TABLE run_provenance",
        "DROP TABLE automation_control",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let s = open(&path);
        let p = s.create_run_provenance(false, None).unwrap();
        drop(s);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(damage).unwrap();
        drop(conn);
        let s = open(&path);
        let restored = s.load_run_provenance(p.id());
        assert!(restored.is_restricted(), "{damage}");
        let generation = restored.stop_generation().expect("stale, not absent");
        assert!(s.automation_commit(generation).is_err(), "{damage}");
        if s.set_automation_stopped(false).is_ok() {
            assert!(s.automation_commit(generation).is_err(), "{damage}");
        }
        assert!(s.update_run_provenance(p.id(), false, None).is_err());
    }
}

#[test]
fn failed_join_rolls_back_and_retry_keeps_both_histories() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s.admit_automation().unwrap();
    let a = s.create_run_provenance(false, None).unwrap();
    let b = s.create_run_provenance(true, Some(old)).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_link BEFORE UPDATE OF parent ON run_provenance
         BEGIN SELECT RAISE(ABORT, 'injected link failure'); END;",
    )
    .unwrap();
    assert!(s.join_run_provenance(a.id(), b.id()).is_err());
    let unchanged = s.load_run_provenance(a.id());
    assert!(unchanged.is_restricted()); // active, even though the update rolled back
    assert!(
        s.automation_commit(unchanged.stop_generation().unwrap())
            .is_err()
    );
    conn.execute_batch("DROP TRIGGER fail_link").unwrap();
    s.join_run_provenance(a.id(), b.id()).unwrap();
    s.seal_run_provenance(a.id()).unwrap();
    for id in [a.id(), b.id()] {
        let joined = s.load_run_provenance(id);
        assert!(joined.is_restricted());
        assert_eq!(joined.stop_generation(), Some(old));
    }
}

#[test]
fn failed_update_is_reported_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let p = s.create_run_provenance(false, None).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_update BEFORE UPDATE ON run_provenance
         BEGIN SELECT RAISE(ABORT, 'injected update failure'); END;",
    )
    .unwrap();
    assert!(s.update_run_provenance(p.id(), true, None).is_err());
    // A crash before retry must not restore the readable old snapshot as clean.
    assert!(s.load_run_provenance(p.id()).is_restricted());
    conn.execute_batch("DROP TRIGGER fail_update").unwrap();
    s.update_run_provenance(p.id(), true, None).unwrap();
    drop(s);
    assert!(open(&path).load_run_provenance(p.id()).is_restricted());
}

#[test]
fn final_snapshot_and_seal_are_one_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s.admit_automation().unwrap();
    let a = s.lease_run_provenance(false, None).unwrap();
    let b = s.lease_run_provenance(false, None).unwrap();
    let ids = [a.id().to_owned(), b.id().to_owned()];
    let leases = [a, b];
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_seal BEFORE UPDATE OF sealed ON run_provenance
         BEGIN SELECT RAISE(ABORT, 'injected seal failure'); END;",
    )
    .unwrap();
    assert!(state::RunProvenanceLease::seal_all(&leases, true, Some(old)).is_err());
    for id in &ids {
        let recovered = s.load_run_provenance(id);
        assert!(!recovered.is_sealed());
        assert!(recovered.is_restricted());
        assert!(
            s.automation_commit(recovered.stop_generation().unwrap())
                .is_err()
        );
    }
    conn.execute_batch("DROP TRIGGER fail_seal").unwrap();
    state::RunProvenanceLease::seal_all(&leases, true, Some(old)).unwrap();
    for id in &ids {
        let recovered = s.load_run_provenance(id);
        assert!(recovered.is_sealed());
        assert!(recovered.is_restricted());
        assert_eq!(recovered.stop_generation(), Some(old));
    }
}

#[test]
fn crash_before_restriction_recovers_conservatively_and_sealed_clean_stays_clean() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let generation = s.admit_automation().unwrap();
    let active = s.create_run_provenance(false, Some(generation)).unwrap();
    let sealed = s.create_run_provenance(false, Some(generation)).unwrap();
    s.seal_run_provenance(sealed.id()).unwrap();
    drop(s); // crash after publishing the identity but before exposure/update
    let s = open(&path);
    let recovered = s.load_run_provenance(active.id());
    assert!(recovered.is_restricted());
    assert!(!recovered.is_sealed());
    assert!(
        s.automation_commit(recovered.stop_generation().unwrap())
            .is_err()
    );
    drop(s.automation_commit(generation).unwrap());
    let clean = s.load_run_provenance(sealed.id());
    assert!(!clean.is_restricted());
    assert!(clean.is_sealed());
    assert_eq!(clean.stop_generation(), Some(generation));
    s.set_automation_stopped(true).unwrap();
    s.set_automation_stopped(false).unwrap();
    drop(s);
    let s = open(&path);
    for id in [active.id(), sealed.id()] {
        let recovered = s.load_run_provenance(id);
        assert!(
            s.automation_commit(recovered.stop_generation().unwrap())
                .is_err()
        );
    }
}

#[test]
fn sealed_component_reopens_when_a_member_changes_or_joins() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let a = s.create_run_provenance(false, None).unwrap();
    let b = s.create_run_provenance(false, None).unwrap();
    s.join_run_provenance(a.id(), b.id()).unwrap();
    s.seal_run_provenance(b.id()).unwrap();
    for id in [a.id(), b.id()] {
        assert!(!s.load_run_provenance(id).is_restricted());
    }
    s.update_run_provenance(b.id(), false, None).unwrap();
    for id in [a.id(), b.id()] {
        assert!(s.load_run_provenance(id).is_restricted());
    }
    s.seal_run_provenance(a.id()).unwrap();
    let c = s.create_run_provenance(false, None).unwrap();
    s.join_run_provenance(c.id(), b.id()).unwrap();
    for id in [a.id(), b.id(), c.id()] {
        assert!(s.load_run_provenance(id).is_restricted());
    }
}

#[test]
fn migration_from_30_preserves_existing_data_and_does_not_invent_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    s.set_meta("existing", "preserved").unwrap();
    let old = s.admit_automation().unwrap();
    s.set_automation_stopped(true).unwrap();
    let job = s.enqueue_trigger("agent", "event", None, "{}", 0).unwrap();
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE run_provenance; PRAGMA user_version=30;")
        .unwrap();
    drop(conn);
    let s = open(&path);
    assert_eq!(s.meta("existing").unwrap().as_deref(), Some("preserved"));
    assert!(s.automation_state().unwrap().stopped);
    assert!(s.automation_commit(old).is_err());
    assert_eq!(s.due_trigger_jobs(1234).unwrap()[0].id, job.unwrap());
    assert!(s.load_run_provenance("legacy").is_restricted());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::EXPECTED_SCHEMA_VERSION);
    let count: i64 = conn
        .query_row("SELECT count(*) FROM run_provenance", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
