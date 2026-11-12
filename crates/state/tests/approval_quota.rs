use state::{ApprovalBudget, ApprovalReservation as R, Clock, State};
use std::sync::{Arc, atomic::{AtomicI64, Ordering}};

struct Time(AtomicI64);
impl Clock for Time {
    fn now_ms(&self) -> i64 { self.0.load(Ordering::SeqCst) }
}
fn budget(n: i64) -> ApprovalBudget {
    ApprovalBudget { rule_daily: n, agent_destructive_daily: Some(n), vault_destructive_daily: Some(n) }
}

#[test]
fn approval_quota_restart_utc_and_uncertain_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let clock = Arc::new(Time(AtomicI64::new(86_400_000 - 1)));
    let s = State::open_with_clock(&path, None, clock.clone()).unwrap();
    assert_eq!(s.reserve_approval("uncertain", "a", "rule", false, budget(1)).unwrap(), R::Reserved);
    s.finish_approval("uncertain", false, false).unwrap();
    drop(s);
    let s = State::open_with_clock(&path, None, clock.clone()).unwrap();
    assert_eq!(s.reserve_approval("second", "a", "rule", false, budget(1)).unwrap(), R::Capped);
    clock.0.store(86_400_000, Ordering::SeqCst);
    assert_eq!(s.reserve_approval("uncertain", "a", "rule", false, budget(1)).unwrap(), R::AlreadyAttempted);
    assert_eq!(s.reserve_approval("second", "a", "rule", false, budget(1)).unwrap(), R::Reserved);
    s.finish_approval("second", false, true).unwrap();
    assert_eq!(s.reserve_approval("third", "a", "rule", false, budget(1)).unwrap(), R::Reserved);
    s.finish_approval("third", true, false).unwrap();
    assert_eq!(s.reserve_approval("second", "a", "rule", false, budget(1)).unwrap(), R::Capped);
}

#[test]
fn approval_quota_destructive_defaults_deny() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = State::open(&path, None).unwrap();
    let missing = ApprovalBudget { rule_daily: 10, agent_destructive_daily: Some(10), vault_destructive_daily: None };
    assert_eq!(s.reserve_approval("off", "a", "r", true, missing).unwrap(), R::Capped);
    let missing = ApprovalBudget { agent_destructive_daily: None, vault_destructive_daily: Some(10), ..missing };
    assert_eq!(s.reserve_approval("off", "a", "r", true, missing).unwrap(), R::Capped);
}

#[test]
fn approval_quota_concurrency_and_agent_buckets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let s = State::open(&path, None).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8).map(|i| {
        let s = State::open(&path, None).unwrap();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            s.reserve_approval(&format!("id{i}"), &format!("agent{i}"), "r", true, budget(3)).unwrap()
        })
    }).collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| **r == R::Reserved).count(), 3);
    let b = ApprovalBudget { rule_daily: 5, agent_destructive_daily: Some(1), vault_destructive_daily: Some(10) };
    assert_eq!(s.reserve_approval("a1", "one-agent", "r1", true, b).unwrap(), R::Reserved);
    assert_eq!(s.reserve_approval("a2", "one-agent", "r2", true, b).unwrap(), R::Capped);
}
