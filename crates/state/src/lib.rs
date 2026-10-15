//! `.vault/state.db`: version history, trash, encrypted secrets, API tokens, paired devices, task
//! run logs, the queue of `after_*` handler jobs and the user's notifications.
//!
//! Unlike `index.db`, nothing here can be rebuilt from the vault files (D25).

mod auth;
mod devices;
mod events;
mod history;
mod jobs;
mod notifications;
mod pins;
mod runs;
mod secrets;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub use auth::{Scope, TokenInfo};
pub use devices::{Device, NewDevice, PLATFORMS, PairCode, PushTarget};
pub use events::{CAPACITY as EVENT_CAPACITY, Events, LiveEvent, Missed, Subscription};
pub use history::{Author, MERGE_GAP_MS, MERGE_SPAN_MS, PrunePolicy, TrashEntry, Version};
pub use jobs::{HandlerJob, JobStatus};
pub use notifications::{
    MAX_BODY, MAX_NOTIFICATIONS, MAX_TITLE, NewNotification, Notification, NotificationPage,
    READ_RETENTION_MS, is_app_path,
};
pub use pins::{PinEntry, PinKind, PinNode};
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
    #[error("no device {0}")]
    UnknownDevice(String),
    #[error("unknown device platform {0:?}; expected linux or android")]
    UnknownPlatform(String),
    #[error("no secret named {0:?}")]
    UnknownSecret(String),
    #[error("no task run {0}")]
    UnknownRun(RunId),
    #[error("task run {0} has already finished")]
    RunFinished(RunId),
    #[error("password hashing failed: {0}")]
    PasswordHash(String),
    #[error("nothing pinned with id {0}")]
    UnknownPin(String),
    #[error("cannot move pin {id} into {target}: it is {id} itself or inside it")]
    PinCycle { id: String, target: String },
    #[error("a folder name cannot be empty")]
    EmptyFolderName,
    #[error("{0}")]
    InvalidNotification(String),
    #[error("no notification {0}")]
    UnknownNotification(String),
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
    events: Events,
}

const SCHEMA_VERSION: i64 = 8;

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

/// What version 2 adds: the durable queue of `after_*` handler jobs (D9).
const SCHEMA_V2: &str = "
CREATE TABLE handler_jobs (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    payload   TEXT NOT NULL,
    queued_ms INTEGER NOT NULL,
    status    TEXT NOT NULL DEFAULT 'queued',
    ended_ms  INTEGER,
    message   TEXT,
    log       TEXT NOT NULL DEFAULT ''
);
CREATE INDEX handler_jobs_status ON handler_jobs (status, id);
";

/// What version 3 adds: facts about the vault itself, such as its id.
const SCHEMA_V3: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// What version 4 adds: the notes pinned to the sidebar, in the order they show.
const SCHEMA_V4: &str = "
CREATE TABLE IF NOT EXISTS pins (
    note_id   TEXT PRIMARY KEY,
    position  INTEGER NOT NULL,
    pinned_ms INTEGER NOT NULL
);
";

/// What version 5 adds: when a version's run of merged browser saves began.
const SCHEMA_V5: &str = "
ALTER TABLE versions ADD COLUMN started_ms INTEGER;
";

/// What version 6 adds: pins become a tree. An entry is a pinned note (its id is the note's id) or
/// a folder (its id is made up, and it has a name). Every entry has a parent, `NULL` for the root,
/// and a position among its siblings. The flat version 4 pins move to the root in their order.
const SCHEMA_V6: &str = "
CREATE TABLE IF NOT EXISTS pin_entries (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('note', 'folder')),
    name       TEXT,
    parent     TEXT,
    position   INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS pin_entries_parent ON pin_entries (parent, position);
INSERT OR IGNORE INTO pin_entries (id, kind, name, parent, position, created_ms)
    SELECT note_id, 'note', NULL, NULL,
           ROW_NUMBER() OVER (ORDER BY position, pinned_ms) - 1, pinned_ms
    FROM pins;
DROP TABLE IF EXISTS pins;
";

/// What version 7 adds: notifications for the user. `id` is a ULID, so it sorts by creation;
/// `read_ms` is `NULL` until the notification is read.
const SCHEMA_V7: &str = "
CREATE TABLE IF NOT EXISTS notifications (
    id         TEXT PRIMARY KEY,
    created_ms INTEGER NOT NULL,
    title      TEXT NOT NULL,
    body       TEXT,
    note_id    TEXT,
    url        TEXT,
    source     TEXT NOT NULL,
    read_ms    INTEGER
);
CREATE INDEX IF NOT EXISTS notifications_unread ON notifications (read_ms, id);
";

/// What version 8 adds: paired devices, each bound to a token of its own that deleting cascades
/// from, and the single-use codes that pair them (D2, D3).
const SCHEMA_V8: &str = "
CREATE TABLE IF NOT EXISTS devices (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    platform      TEXT NOT NULL,
    capabilities  TEXT NOT NULL,
    token_id      TEXT NOT NULL UNIQUE REFERENCES tokens (name) ON DELETE CASCADE,
    push_endpoint TEXT,
    push_p256dh   TEXT,
    push_auth     TEXT,
    last_seen_ms  INTEGER,
    settings      TEXT NOT NULL,
    created_ms    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pair_codes (
    code_hash  BLOB PRIMARY KEY,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL
);
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
            events: Events::default(),
        })
    }

    /// Use `key` for secrets from now on, such as once [`State::vault_id`] has named the key file.
    pub fn with_key(mut self, key: Option<SecretKey>) -> Self {
        self.key = key;
        self
    }

    /// The vault's id, which names its secret key file (OQ-S2). The first call makes one up and
    /// stores it, so the id travels with `state.db` into a backup.
    pub fn vault_id(&self) -> Result<String> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('vault_id', ?1)",
            [random_hex(16)],
        )?;
        Ok(
            conn.query_row("SELECT value FROM meta WHERE key = 'vault_id'", [], |r| {
                r.get(0)
            })?,
        )
    }

    /// Write a consistent copy of the whole database to `path`, which must not exist yet. Safe
    /// while another process has `state.db` open.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        let target = path
            .to_str()
            .ok_or_else(|| Error::Corrupt(format!("{} is not UTF-8", path.display())))?;
        self.conn().execute("VACUUM INTO ?1", [target])?;
        Ok(())
    }

    /// The live events of this vault's changes, for open clients (D4).
    pub fn events(&self) -> &Events {
        &self.events
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    /// A new ULID for the current time.
    fn new_ulid(&self) -> ulid::Ulid {
        let mut random = [0u8; 10];
        random_bytes(&mut random);
        let random = random
            .iter()
            .fold(0u128, |acc, b| (acc << 8) | u128::from(*b));
        ulid::Ulid::from_parts(u64::try_from(self.now()).unwrap_or(0), random)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match version {
        0..=7 => {
            let tx = conn.transaction()?;
            if version == 0 {
                tx.execute_batch(SCHEMA)?;
            }
            if version <= 1 {
                tx.execute_batch(SCHEMA_V2)?;
            }
            if version <= 2 {
                tx.execute_batch(SCHEMA_V3)?;
            }
            if version <= 3 {
                tx.execute_batch(SCHEMA_V4)?;
            }
            let has_started: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM pragma_table_info('versions') WHERE name = 'started_ms')",
                [],
                |r| r.get(0),
            )?;
            if !has_started {
                tx.execute_batch(SCHEMA_V5)?;
            }
            if version <= 5 {
                tx.execute_batch(SCHEMA_V6)?;
            }
            if version <= 6 {
                tx.execute_batch(SCHEMA_V7)?;
            }
            if version <= 7 {
                tx.execute_batch(SCHEMA_V8)?;
            }
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

pub(crate) fn random_bytes(buf: &mut [u8]) {
    use chacha20poly1305::aead::OsRng;
    use chacha20poly1305::aead::rand_core::RngCore;
    OsRng.fill_bytes(buf);
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
