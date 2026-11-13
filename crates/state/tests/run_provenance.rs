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
fn schema_35_recovery_tracks_reads_but_new_independent_runs_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s.create_run_provenance(false, Some(s.admit_automation().unwrap())).unwrap();
    s.seal_run_provenance(old.id()).unwrap();
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("ALTER TABLE run_provenance DROP COLUMN agent_reads; PRAGMA user_version=35;").unwrap();
    drop(conn);
    let s = open(&path);
    let restored = s.load_run_provenance(old.id());
    assert!(restored.tracks_agent_reads());
    assert!(!restored.is_restricted());
    let new = s.create_run_provenance(false, Some(s.admit_automation().unwrap())).unwrap();
    s.seal_run_provenance(new.id()).unwrap();
    assert!(!s.load_run_provenance(new.id()).tracks_agent_reads());
    s.join_run_provenance(new.id(), old.id()).unwrap();
    s.seal_run_provenance(new.id()).unwrap();
    assert!(s.load_run_provenance(new.id()).tracks_agent_reads());
}

#[test]
fn schema_34_handler_upgrade_preserves_payloads_and_atomic_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s.queue_jobs(&["legacy".into()]).unwrap()[0];
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE handler_provenance; DROP TABLE outbox_provenance; PRAGMA user_version=34;",
    )
    .unwrap();
    drop(conn);
    let s = open(&path);
    assert_eq!(s.queued_jobs().unwrap()[0].payload, "legacy");
    assert_eq!(s.handler_provenance(old).unwrap(), None);
    let lease = s
        .lease_run_provenance(true, Some(s.admit_automation().unwrap()))
        .unwrap();
    let id = s
        .queue_jobs_with_provenance(&["queued".into()], lease.id())
        .unwrap()[0];
    assert_eq!(
        s.handler_provenance(id).unwrap().as_deref(),
        Some(lease.id())
    );
    s.append_outbox_provenance("item", lease.id(), true)
        .unwrap();
    s.append_outbox_provenance("item", "later", false).unwrap();
    assert_eq!(s.outbox_provenance("item").unwrap().len(), 2);
    s.finish_job(id, Ok(()), "").unwrap();
    assert!(s.handler_provenance(id).unwrap().is_none());
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_handler_reference BEFORE INSERT ON handler_provenance BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(
        s.queue_jobs_with_provenance(&["not published".into()], lease.id())
            .is_err()
    );
    assert_eq!(s.queued_jobs().unwrap().len(), 1);
}

#[test]
fn schema_33_upgrade_and_dedup_keep_all_queue_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let old = s
        .enqueue_trigger("a", "t", Some("legacy"), "old", 0)
        .unwrap()
        .unwrap();
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE trigger_provenance; PRAGMA user_version=33;")
        .unwrap();
    drop(conn);
    let s = open(&path);
    assert_eq!(s.due_trigger_jobs(0).unwrap()[0].id, old);
    assert!(s.trigger_provenance(old).unwrap().is_empty());
    let first = s
        .lease_run_provenance(true, Some(s.admit_automation().unwrap()))
        .unwrap();
    let second = s
        .lease_run_provenance(false, Some(s.admit_automation().unwrap()))
        .unwrap();
    let (id, inserted) = s
        .enqueue_trigger_with_provenance("a", "t", Some("new"), "first", 0, false, first.id())
        .unwrap()
        .unwrap();
    assert!(inserted);
    assert_eq!(
        s.enqueue_trigger_with_provenance("a", "t", Some("new"), "second", 0, false, second.id())
            .unwrap(),
        Some((id, false))
    );
    let ids = s.trigger_provenance(id).unwrap();
    assert!(ids.contains(&first.id().to_owned()));
    assert!(ids.contains(&second.id().to_owned()));
    s.enqueue_trigger_with_provenance("a", "t", Some("legacy"), "new", 0, false, second.id())
        .unwrap();
    assert!(s.trigger_provenance(old).unwrap().contains(&String::new()));
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_ref BEFORE INSERT ON trigger_provenance BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(
        s.enqueue_trigger_with_provenance("a", "t", Some("new"), "lost", 0, false, first.id())
            .is_err()
    );
    assert_eq!(
        s.due_trigger_jobs(0)
            .unwrap()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap()
            .payload,
        "second"
    );
    assert!(
        s.enqueue_trigger_with_provenance("a", "t", Some("rollback"), "lost", 0, false, first.id())
            .is_err()
    );
    assert_eq!(s.due_trigger_jobs(0).unwrap().len(), 2);
}

#[test]
fn failed_seal_is_visible_and_audit_failure_keeps_recovery_fallible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let lease = s.lease_run_provenance(false, None).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_seal BEFORE UPDATE ON run_provenance
         BEGIN SELECT RAISE(ABORT, 'injected seal failure'); END;",
    )
    .unwrap();
    assert!(state::RunProvenanceLease::seal_all(&[lease], false, None).is_err());
    // A fresh lease still records to activity even when provenance updates fail.
    let lease = s.lease_run_provenance(false, None).unwrap();
    lease.record_seal_failure().unwrap();
    let activity = s.activity(&state::ActivityFilter::default()).unwrap();
    assert_eq!(activity.items[0].kind, "run.seal_failed");
    assert_eq!(activity.items[0].at_ms, 1234);
    assert!(!s.load_run_provenance(lease.id()).is_sealed());
    conn.execute_batch(
        "CREATE TRIGGER fail_audit BEFORE INSERT ON activity
         BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
    )
    .unwrap();
    assert!(s.record_unavailable_run("missing").is_err());
    assert!(lease.record_seal_failure().is_err());
}

#[test]
fn joined_component_waits_for_both_live_owners_across_connections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let other = open(&path);
    let a = s.create_run_provenance(false, None).unwrap();
    let b = s.create_run_provenance(false, None).unwrap();
    s.join_run_provenance(a.id(), b.id()).unwrap();
    s.seal_run_provenance(a.id()).unwrap();
    let (_, left) = s.resume_run_provenance(a.id()).unwrap();
    let (_, right) = other.resume_run_provenance(b.id()).unwrap();
    assert!(s.seal_run_provenance(a.id()).is_err());
    state::RunProvenanceLease::seal_all(&[left], false, None).unwrap();
    for id in [a.id(), b.id()] {
        let active = s.load_run_provenance(id);
        assert!(!active.is_sealed());
        assert!(active.is_restricted());
        assert!(
            s.automation_commit(active.stop_generation().unwrap())
                .is_err()
        );
    }
    state::RunProvenanceLease::seal_all(&[right], true, None).unwrap();
    for id in [a.id(), b.id()] {
        assert!(s.load_run_provenance(id).is_sealed());
        assert!(s.load_run_provenance(id).is_restricted());
    }
}

#[test]
fn abandoned_lease_cannot_be_sealed_by_another_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let lease = s.lease_run_provenance(false, None).unwrap();
    let id = lease.id().to_owned();
    std::mem::forget(lease);
    drop(s);
    let s = open(&path);
    let (_, second) = s.resume_run_provenance(&id).unwrap();
    state::RunProvenanceLease::seal_all(&[second], false, None).unwrap();
    assert!(!s.load_run_provenance(&id).is_sealed());
    assert!(s.seal_run_provenance(&id).is_err());
}

#[test]
fn migration_from_32_preserves_sealed_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = open(&path);
    let p = s.create_run_provenance(false, None).unwrap();
    s.seal_run_provenance(p.id()).unwrap();
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE run_provenance_leases; PRAGMA user_version=32;")
        .unwrap();
    drop(conn);
    let s = open(&path);
    assert!(s.load_run_provenance(p.id()).is_sealed());
    assert!(!s.load_run_provenance(p.id()).is_restricted());
    let (_, lease) = s.resume_run_provenance(p.id()).unwrap();
    state::RunProvenanceLease::seal_all(&[lease], false, None).unwrap();
    assert!(s.load_run_provenance(p.id()).is_sealed());
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
