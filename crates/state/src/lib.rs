//! `.vault/state.db`: version history, trash, encrypted secrets, API tokens and task run logs.
//!
//! Unlike `index.db`, nothing here can be rebuilt from the vault files (D25).

mod auth;
mod history;
mod runs;
mod secrets;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub use auth::{Scope, TokenInfo};
pub use history::{Author, PrunePolicy, TrashEntry, Version};
pub use runs::{RunId, RunOutcome, TaskRun};
pub use secrets::{SecretInfo, SecretKey, key_file_path};

/// Errors from `state.db`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("state.db: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("state.db: {0}")]
    Io(#[from] std::io::Error),
    #[error("secret key is missing: {}", match path {
        Some(p) => format!("no key file at {}", p.display()),
        None => "state.db was opened without a key file".to_owned(),
    })]
    KeyMissing { path: Option<PathBuf> },
    #[error("secret key is wrong: {detail}")]
    WrongKey { detail: String },
    #[error("note {0} has no recorded versions")]
    UnknownNote(String),
    #[error("note {0} is not in the trash")]
    NotInTrash(String),
    #[error("invalid prune policy: {0}")]
    InvalidPrunePolicy(String),
    #[error("a token named {0:?} already exists")]
    TokenExists(String),
    #[error("no token named {0:?}")]
    UnknownToken(String),
    #[error("unknown token scope {0:?}; expected read, write or admin")]
    UnknownScope(String),
    #[error("no secret named {0:?}")]
    UnknownSecret(String),
    #[error("no task run {0}")]
    UnknownRun(RunId),
    #[error("task run {0} has already finished")]
    RunFinished(RunId),
    #[error("password hashing failed: {0}")]
    PasswordHash(String),
    #[error("state.db is corrupt: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Source of the current time, in milliseconds since the Unix epoch. Tests swap in their own.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The real wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// An open `state.db`.
pub struct State {
    conn: Mutex<Connection>,
    key: Option<SecretKey>,
    clock: Arc<dyn Clock>,
}

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE versions (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id     TEXT NOT NULL,
    note_type   TEXT NOT NULL,
    token       TEXT NOT NULL UNIQUE,
    author_kind TEXT NOT NULL,
    author_name TEXT NOT NULL,
    at_ms       INTEGER NOT NULL,
    content     TEXT NOT NULL
);
CREATE INDEX versions_note ON versions (note_id, seq);

-- The current version of every live note. A trashed note has no row.
CREATE TABLE heads (
    note_id TEXT PRIMARY KEY,
    token   TEXT NOT NULL
);

CREATE TABLE trash (
    note_id     TEXT PRIMARY KEY,
    note_type   TEXT NOT NULL,
    content     TEXT NOT NULL,
    version     TEXT NOT NULL,
    author_kind TEXT NOT NULL,
    author_name TEXT NOT NULL,
    deleted_ms  INTEGER NOT NULL
);

CREATE TABLE tokens (
    name         TEXT PRIMARY KEY,
    scope        TEXT NOT NULL,
    secret_hash  BLOB NOT NULL UNIQUE,
    created_ms   INTEGER NOT NULL,
    last_used_ms INTEGER
);

CREATE TABLE password (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    hash TEXT NOT NULL
);

CREATE TABLE secrets (
    name       TEXT PRIMARY KEY,
    nonce      BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);

CREATE TABLE task_runs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task       TEXT NOT NULL,
    attempt    INTEGER NOT NULL,
    started_ms INTEGER NOT NULL,
    ended_ms   INTEGER,
    outcome    TEXT NOT NULL,
    message    TEXT,
    log        TEXT NOT NULL DEFAULT ''
);
CREATE INDEX task_runs_task ON task_runs (task, id);
";

impl State {
    /// Opens or creates `state.db` at `path`. Without `key`, secrets can be listed but not
    /// read or written.
    pub fn open(path: &Path, key: Option<SecretKey>) -> Result<Self> {
        Self::open_with_clock(path, key, Arc::new(SystemClock))
    }

    /// Like [`State::open`], reading time from `clock`.
    pub fn open_with_clock(
        path: &Path,
        key: Option<SecretKey>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        // Overwrite deleted rows so pruned history and removed secrets do not linger on disk.
        conn.pragma_update(None, "secure_delete", true)?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            key,
            clock,
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> i64 {
        self.clock.now_ms()
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match version {
        0 => {
            let tx = conn.transaction()?;
            tx.execute_batch(SCHEMA)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
            Ok(())
        }
        SCHEMA_VERSION => Ok(()),
        other => Err(Error::Corrupt(format!(
            "schema version {other} is newer than this build supports ({SCHEMA_VERSION})"
        ))),
    }
}

/// Fills `buf` from the operating system's random source.
/// `bytes` random bytes from the operating system, as lowercase hex. For session ids and other
/// secrets outside `state.db`.
pub fn random_hex(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    random_bytes(&mut raw);
    to_hex(&raw)
}

fn random_bytes(buf: &mut [u8]) {
    use chacha20poly1305::aead::OsRng;
    use chacha20poly1305::aead::rand_core::RngCore;
    OsRng.fill_bytes(buf);
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
