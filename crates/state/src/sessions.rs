//! Password-login sessions (OQ-S3). A session id is 256 random bits handed to the browser in a
//! cookie; only its SHA-256 is stored, so reading `state.db` gives no one a way in.
//!
//! A session lasts [`SESSION_TTL_MS`] from the last visit: a check more than [`SESSION_RENEW_MS`]
//! after the last renewal moves the expiry forward, so an active session never runs out and checks
//! within the hour write nothing. Changing the password ends every session.

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{Result, State, random_hex, to_hex};

/// How long a session lasts after its last renewal: 30 days.
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// How long after its last renewal a session in use is renewed again: an hour.
pub const SESSION_RENEW_MS: i64 = 60 * 60 * 1000;

/// The longest user agent kept with a session, in characters.
const MAX_USER_AGENT: usize = 200;

/// A live session, as [`State::check_session`] found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCheck {
    pub expires_ms: i64,
    /// Whether this check renewed the session, so the cookie should be sent again.
    pub renewed: bool,
}

fn id_hash(id: &str) -> String {
    to_hex(&Sha256::digest(id.as_bytes()))
}

/// Read-only credential checks for a long-lived response. Keeps only a hash, never the secret.
#[derive(Clone)]
pub struct StreamCredential {
    conn: std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    clock: std::sync::Arc<dyn crate::Clock>,
    hash: rusqlite::types::Value,
    token: bool,
    pub changes: tokio::sync::watch::Receiver<()>,
}

impl StreamCredential {
    /// Does not renew a session or record token/device usage.
    pub fn is_valid(&self) -> Result<bool> {
        let sql = if self.token {
            "SELECT EXISTS(SELECT 1 FROM tokens WHERE secret_hash = ?1 AND (expires_ms IS NULL OR expires_ms > ?2))"
        } else {
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id_hash = ?1 AND expires_ms > ?2)"
        };
        Ok(self.conn.lock().unwrap_or_else(|e| e.into_inner()).query_row(
            sql,
            params![self.hash, self.clock.now_ms()],
            |r| r.get(0),
        )?)
    }
}

impl State {
    /// Subscribe before starting a response, so a concurrent revocation cannot be missed.
    pub fn stream_credential(&self, secret: &str, token: bool) -> StreamCredential {
        StreamCredential {
            conn: self.conn.clone(),
            clock: self.clock.clone(),
            hash: if token {
                rusqlite::types::Value::Blob(Sha256::digest(secret.as_bytes()).to_vec())
            } else {
                rusqlite::types::Value::Text(id_hash(secret))
            },
            token,
            changes: self.credential_changes.subscribe(),
        }
    }
    /// Starts a session and returns its id, which is shown once, and when it expires. Expired
    /// sessions are removed on the way.
    pub fn create_session(&self, user_agent: Option<&str>) -> Result<(String, i64)> {
        let id = random_hex(32);
        let now = self.now();
        let expires_ms = now + SESSION_TTL_MS;
        let user_agent = user_agent
            .map(str::trim)
            .filter(|ua| !ua.is_empty())
            .map(|ua| ua.chars().take(MAX_USER_AGENT).collect::<String>());
        let conn = self.conn();
        conn.execute("DELETE FROM sessions WHERE expires_ms <= ?1", [now])?;
        conn.execute(
            "INSERT INTO sessions (id_hash, created_ms, last_seen_ms, expires_ms, user_agent)
             VALUES (?1, ?2, ?2, ?3, ?4)",
            params![id_hash(&id), now, expires_ms, user_agent],
        )?;
        Ok((id, expires_ms))
    }

    /// Whether session `id` is live, renewing it when its last renewal was more than
    /// [`SESSION_RENEW_MS`] ago. An expired session is removed and is `None`.
    pub fn check_session(&self, id: &str) -> Result<Option<SessionCheck>> {
        let now = self.now();
        let hash = id_hash(id);
        let conn = self.conn();
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT last_seen_ms, expires_ms FROM sessions WHERE id_hash = ?1",
                [&hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((last_seen_ms, expires_ms)) = row else {
            return Ok(None);
        };
        if expires_ms <= now {
            conn.execute("DELETE FROM sessions WHERE id_hash = ?1", [&hash])?;
            return Ok(None);
        }
        if now - last_seen_ms <= SESSION_RENEW_MS {
            return Ok(Some(SessionCheck {
                expires_ms,
                renewed: false,
            }));
        }
        let expires_ms = now + SESSION_TTL_MS;
        conn.execute(
            "UPDATE sessions SET last_seen_ms = ?1, expires_ms = ?2 WHERE id_hash = ?3",
            params![now, expires_ms, hash],
        )?;
        Ok(Some(SessionCheck {
            expires_ms,
            renewed: true,
        }))
    }

    /// Ends session `id`. Answers whether it existed.
    pub fn end_session(&self, id: &str) -> Result<bool> {
        let removed = self
            .conn()
            .execute("DELETE FROM sessions WHERE id_hash = ?1", [id_hash(id)])?;
        self.credential_changes.send_replace(());
        Ok(removed > 0)
    }

    /// Ends every session, logging every browser out. Answers how many there were.
    pub fn end_all_sessions(&self) -> Result<usize> {
        let removed = self.conn().execute("DELETE FROM sessions", [])?;
        self.credential_changes.send_replace(());
        Ok(removed)
    }

    /// Removes the sessions that have expired. Answers how many.
    pub fn prune_sessions(&self) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM sessions WHERE expires_ms <= ?1", [self.now()])?)
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicI64, Ordering}};

    struct Clock(AtomicI64);
    impl crate::Clock for Clock {
        fn now_ms(&self) -> i64 { self.0.load(Ordering::SeqCst) }
    }

    #[test]
    fn stream_check_does_not_renew_and_recovers_poison() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock(AtomicI64::new(1_000)));
        let db = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();
        let (id, expires) = db.create_session(None).unwrap();
        let credential = db.stream_credential(&id, false);
        clock.0.store(1_000 + SESSION_RENEW_MS + 1, Ordering::SeqCst);
        assert!(credential.is_valid().unwrap());
        let row: (i64, i64) = db.conn().query_row("SELECT last_seen_ms, expires_ms FROM sessions", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(row, (1_000, expires));
        let conn = db.conn.clone();
        assert!(std::thread::spawn(move || {
            let _lock = conn.lock().unwrap();
            panic!("poison test");
        }).join().is_err());
        assert!(credential.is_valid().unwrap());
    }
}
