use rusqlite::Connection;
use state::{ApprovalBudget, ApprovalReservation as R, Clock, Scope, State, TokenInfo};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct Time(AtomicI64);
impl Clock for Time {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
fn budget(n: i64) -> ApprovalBudget {
    ApprovalBudget {
        rule_daily: n,
        agent_destructive_daily: Some(n),
        vault_destructive_daily: Some(n),
    }
}

#[test]
fn approval_quota_restart_utc_and_uncertain_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let clock = Arc::new(Time(AtomicI64::new(86_400_000 - 1)));
    let s = State::open_with_clock(&path, None, clock.clone()).unwrap();
    assert_eq!(
        s.reserve_approval("uncertain", "a", "rule", false, budget(1))
            .unwrap(),
        R::Reserved
    );
    s.finish_approval("uncertain", false, false).unwrap();
    drop(s);
    let s = State::open_with_clock(&path, None, clock.clone()).unwrap();
    assert_eq!(
        s.reserve_approval("second", "a", "rule", false, budget(1))
            .unwrap(),
        R::Capped
    );
    clock.0.store(86_400_000, Ordering::SeqCst);
    assert_eq!(
        s.reserve_approval("uncertain", "a", "rule", false, budget(1))
            .unwrap(),
        R::AlreadyAttempted
    );
    assert_eq!(
        s.reserve_approval("second", "a", "rule", false, budget(1))
            .unwrap(),
        R::Reserved
    );
    s.finish_approval("second", false, true).unwrap();
    assert_eq!(
        s.reserve_approval("third", "a", "rule", false, budget(1))
            .unwrap(),
        R::Reserved
    );
    s.finish_approval("third", true, false).unwrap();
    assert_eq!(
        s.reserve_approval("second", "a", "rule", false, budget(1))
            .unwrap(),
        R::Capped
    );
}

#[test]
fn approval_quota_destructive_defaults_deny() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = State::open(&path, None).unwrap();
    let missing = ApprovalBudget {
        rule_daily: 10,
        agent_destructive_daily: Some(10),
        vault_destructive_daily: None,
    };
    assert_eq!(
        s.reserve_approval("off", "a", "r", true, missing).unwrap(),
        R::Capped
    );
    let missing = ApprovalBudget {
        agent_destructive_daily: None,
        vault_destructive_daily: Some(10),
        ..missing
    };
    assert_eq!(
        s.reserve_approval("off", "a", "r", true, missing).unwrap(),
        R::Capped
    );
}

#[test]
fn approval_quota_gains_its_table_when_a_schema_27_state_db_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    // A production alpha.39 state.db has schema 27: schema 28 minus the
    // approval_quota table and its day index, with rows already in place.
    drop(State::open(&path, None).unwrap());
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE approval_quota;
             INSERT INTO tokens (name, scope, secret_hash, created_ms, kind)
                 VALUES ('agent-27', 'write', x'ab', 111, 'agent');
             INSERT INTO agent_call_ids (call, note, agent, principal, expires_ms, used, scope)
                 VALUES ('call-27', 'note-27', 'agent-27', 'session:u', 222, 0, 'write');
             PRAGMA user_version = 27;",
        )
        .unwrap();
    }
    let state = State::open(&path, None).unwrap();
    assert_eq!(
        state.list_tokens().unwrap(),
        [TokenInfo {
            kind: "agent".to_string(),
            name: "agent-27".to_string(),
            scope: Scope::Write,
            created_ms: 111,
            last_used_ms: None,
        }]
    );
    drop(state);
    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::EXPECTED_SCHEMA_VERSION);
    let quota_columns: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('approval_quota')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(quota_columns, 6);
    let day_index: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'approval_quota_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(day_index, 1);
    let call: (String, String, i64, String) = conn
        .query_row(
            "SELECT call, note, used, scope FROM agent_call_ids",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        call,
        (
            "call-27".to_string(),
            "note-27".to_string(),
            0,
            "write".to_string()
        )
    );
    let quota_rows: i64 = conn
        .query_row("SELECT count(*) FROM approval_quota", [], |r| r.get(0))
        .unwrap();
    assert_eq!(quota_rows, 0);
}

#[test]
fn approval_quota_concurrency_and_agent_buckets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = State::open(&path, None).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let s = State::open(&path, None).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                s.reserve_approval(
                    &format!("id{i}"),
                    &format!("agent{i}"),
                    "r",
                    true,
                    budget(3),
                )
                .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| **r == R::Reserved).count(), 3);
    let b = ApprovalBudget {
        rule_daily: 5,
        agent_destructive_daily: Some(1),
        vault_destructive_daily: Some(10),
    };
    assert_eq!(
        s.reserve_approval("a1", "one-agent", "r1", true, b)
            .unwrap(),
        R::Reserved
    );
    assert_eq!(
        s.reserve_approval("a2", "one-agent", "r2", true, b)
            .unwrap(),
        R::Capped
    );
}
