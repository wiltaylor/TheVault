//! Password-login sessions in `state.db`: they survive reopening, expire, renew while in use, are
//! stored only as hashes, and end when the password changes.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use state::{Clock, SESSION_RENEW_MS, SESSION_TTL_MS, SessionCheck, State};
use tempfile::TempDir;

struct TestClock(AtomicI64);

impl TestClock {
    fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

const START: i64 = 1_000_000;

fn open(dir: &TempDir, clock: &Arc<TestClock>) -> State {
    State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap()
}

fn clock() -> Arc<TestClock> {
    Arc::new(TestClock(AtomicI64::new(START)))
}

fn rows(dir: &TempDir) -> Vec<(String, i64, i64, i64, Option<String>)> {
    let conn = Connection::open(dir.path().join("state.db")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id_hash, created_ms, last_seen_ms, expires_ms, user_agent FROM sessions
             ORDER BY created_ms",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn a_session_survives_reopening_state_db() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    let (id, expires) = state.create_session(Some("Firefox")).unwrap();
    assert_eq!(expires, START + SESSION_TTL_MS);
    drop(state);

    let state = open(&dir, &clock);
    assert_eq!(
        state.check_session(&id).unwrap(),
        Some(SessionCheck {
            expires_ms: expires,
            renewed: false
        })
    );
    assert_eq!(state.check_session("not a session").unwrap(), None);
}

#[test]
fn the_stored_row_holds_a_hash_of_the_id_not_the_id() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    let long_agent = format!("  {}  ", "x".repeat(300));
    let (id, _) = state.create_session(Some(&long_agent)).unwrap();
    drop(state);

    let rows = rows(&dir);
    assert_eq!(rows.len(), 1);
    let (hash, created, seen, expires, agent) = &rows[0];
    let want: String = Sha256::digest(id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hash, &want);
    assert_ne!(hash, &id);
    assert_eq!(
        (*created, *seen, *expires),
        (START, START, START + SESSION_TTL_MS)
    );
    assert_eq!(agent.as_deref(), Some("x".repeat(200).as_str()));

    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        bytes.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }
    assert!(!bytes.windows(id.len()).any(|w| w == id.as_bytes()));
}

#[test]
fn ending_a_session_refuses_it() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    let (a, _) = state.create_session(None).unwrap();
    let (b, _) = state.create_session(None).unwrap();
    assert!(state.end_session(&a).unwrap());
    assert!(!state.end_session(&a).unwrap());
    assert_eq!(state.check_session(&a).unwrap(), None);
    assert!(state.check_session(&b).unwrap().is_some());
}

#[test]
fn an_expired_session_is_refused_and_removed() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    let (id, _) = state.create_session(None).unwrap();
    clock.advance(SESSION_TTL_MS);
    assert_eq!(state.check_session(&id).unwrap(), None);
    assert!(rows(&dir).is_empty());
}

#[test]
fn a_session_in_use_after_the_renewal_hour_moves_its_expiry() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    let (id, first) = state.create_session(None).unwrap();

    // Within the hour nothing moves.
    clock.advance(SESSION_RENEW_MS);
    assert_eq!(
        state.check_session(&id).unwrap(),
        Some(SessionCheck {
            expires_ms: first,
            renewed: false
        })
    );

    clock.advance(1);
    let now = START + SESSION_RENEW_MS + 1;
    assert_eq!(
        state.check_session(&id).unwrap(),
        Some(SessionCheck {
            expires_ms: now + SESSION_TTL_MS,
            renewed: true
        })
    );
    let (_, _, seen, expires, _) = rows(&dir).remove(0);
    assert_eq!((seen, expires), (now, now + SESSION_TTL_MS));
    // The next check in the same hour does not renew it again.
    assert!(!state.check_session(&id).unwrap().unwrap().renewed);

    // Used every few weeks, it never runs out.
    for _ in 0..3 {
        clock.advance(SESSION_TTL_MS - 1);
        assert!(state.check_session(&id).unwrap().unwrap().renewed);
    }
}

#[test]
fn expired_sessions_are_pruned() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    state.create_session(None).unwrap();
    clock.advance(SESSION_TTL_MS / 2);
    let (fresh, _) = state.create_session(None).unwrap();
    clock.advance(SESSION_TTL_MS / 2);
    assert_eq!(state.prune_sessions().unwrap(), 1);
    assert_eq!(rows(&dir).len(), 1);
    assert!(state.check_session(&fresh).unwrap().is_some());

    // Starting a session prunes too.
    clock.advance(SESSION_TTL_MS);
    state.create_session(None).unwrap();
    assert_eq!(rows(&dir).len(), 1);
}

#[test]
fn changing_the_password_ends_every_session() {
    let dir = TempDir::new().unwrap();
    let clock = clock();
    let state = open(&dir, &clock);
    state.set_password("first").unwrap();
    let (a, _) = state.create_session(None).unwrap();
    let (b, _) = state.create_session(None).unwrap();
    state.set_password("second").unwrap();
    assert_eq!(state.check_session(&a).unwrap(), None);
    assert_eq!(state.check_session(&b).unwrap(), None);

    state.create_session(None).unwrap();
    state.create_session(None).unwrap();
    assert_eq!(state.end_all_sessions().unwrap(), 2);
    assert!(rows(&dir).is_empty());
}
