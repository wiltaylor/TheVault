//! What the device remembers between syncs, in its local `state.db` (D17, D29).
//!
//! The tables are the sync engine's own, all named `sync_*`, made on open with
//! `CREATE TABLE IF NOT EXISTS`. They sit beside the server tables of the local vault, which know
//! nothing of them:
//!
//! - `sync_queue`: one row per note changed locally and not yet on the server, in the order of
//!   its first change, with the base it started from: the server version, and that version's WCL
//!   text, which the three-way merge needs whatever the server has done with its own copy since.
//! - `sync_notes`: per note, the server version the local copy matches, the local version that
//!   holds it (so the base text of the next local change can be found in the local history), and
//!   whether the server deleted it.
//! - `sync_assets`: every asset the server or the device holds, its size, whether the server has
//!   it, and when the device last used it, for the asset policy (D19).
//! - `sync_asset_failures`: each asset the device holds that failed to reach the server, why, how
//!   many times in a row, and when the next round tries it again (ticket 141).
//! - `sync_held`: each queued note the last round kept back, because it names an asset that has
//!   not reached the server yet, which the server would refuse it for, or because the server
//!   refused the note itself (ticket 143). A note held for its own sake has an empty `asset_id`.
//! - `sync_recent`: notes opened on the device, for the on-demand asset policy.
//! - `sync_meta`: the change-feed cursor, the id of the vault on the home server the local vault
//!   is a copy of, and the record of the last fresh start (ticket 143).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use writer::Saved;

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS sync_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_notes (
    note_id        TEXT PRIMARY KEY,
    server_version TEXT NOT NULL,
    local_version  TEXT,
    deleted        INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_queue (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id      TEXT NOT NULL UNIQUE,
    base_version TEXT,
    base_wcl     TEXT,
    base_deleted INTEGER NOT NULL,
    queued_ms    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_assets (
    asset_id  TEXT PRIMARY KEY,
    filename  TEXT NOT NULL,
    size      INTEGER NOT NULL,
    on_server INTEGER NOT NULL,
    used_ms   INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_asset_failures (
    asset_id TEXT PRIMARY KEY,
    filename TEXT NOT NULL,
    size     INTEGER NOT NULL,
    error    TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    last_ms  INTEGER NOT NULL,
    next_ms  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_held (
    note_id  TEXT PRIMARY KEY,
    asset_id TEXT NOT NULL,
    reason   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_recent (
    note_id   TEXT PRIMARY KEY,
    opened_ms INTEGER NOT NULL
);
";

/// How long a statement waits for the server half of the process, which writes `state.db`
/// through its own connection.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The cursor key in `sync_meta`.
const CURSOR: &str = "cursor";

/// The `sync_meta` key of the home server vault's id, from its `GET /api/sync/info`.
const SERVER_VAULT: &str = "server_vault";

/// The `sync_meta` key of the last fresh start, as JSON.
const FRESH_START: &str = "fresh_start";

/// The `sync_meta` key of the vault whose contact links the phone was last cleaned up for.
const LINKS_CLEANED: &str = "contact_links_cleaned";

/// A local change waiting to reach the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub note_id: String,
    /// The server version the change started from. `None` for a note the server has never had.
    pub base_version: Option<String>,
    /// The WCL text of that version, when the device still has it.
    pub base_wcl: Option<String>,
    /// The server had deleted the note when the change started: pushing it restores the note.
    pub base_deleted: bool,
}

/// Where the local copy of a note stands against the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synced {
    pub server_version: String,
    /// The local version holding the server's text, `None` when the local copy has moved on.
    pub local_version: Option<String>,
    pub deleted: bool,
}

/// One asset the sync engine knows of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownAsset {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub on_server: bool,
    pub used_ms: i64,
}

/// An asset the device holds that has not reached the server: the last try failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFailure {
    pub id: String,
    pub filename: String,
    pub size: u64,
    /// Why the last try failed.
    pub error: String,
    /// The tries that failed in a row.
    pub attempts: u32,
    /// When the last try failed, in milliseconds since the epoch.
    pub last_ms: i64,
    /// When the next round tries it again.
    pub next_ms: i64,
}

/// A queued note kept back from the server: until an asset it names gets there, or because the
/// server refused the note itself (ticket 143). Either way the rest of the queue goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub note_id: String,
    /// The asset the note waits for; `None` when the server refused the note itself.
    pub asset_id: Option<String>,
    pub reason: String,
}

/// The device's sync state. Opened on the local `state.db`.
pub struct Queue {
    conn: Mutex<Connection>,
}

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Queue {
    /// Open the sync tables in the `state.db` at `path`, making them when they are missing.
    pub fn open(path: &Path) -> Result<Queue> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(TABLES)?;
        Ok(Queue {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue the notes of a committed local write. A note already queued keeps the base its first
    /// change started from; the push sends whatever the note holds when it runs. The base text is
    /// read from `state`, the local history, where the local version that held the server's text
    /// is kept.
    pub fn record(&self, state: &state::State, saved: &[Saved]) -> Result<()> {
        let conn = self.conn();
        for s in saved {
            let queued: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM sync_queue WHERE note_id = ?1)",
                [&s.id],
                |r| r.get(0),
            )?;
            if queued {
                continue;
            }
            let synced = synced_row(&conn, &s.id)?;
            let (base_version, base_wcl, base_deleted) = match synced {
                None => (None, None, false),
                Some(row) => {
                    let text = row
                        .local_version
                        .as_deref()
                        .and_then(|v| state.version(&s.id, v).ok().flatten())
                        .map(|v| v.content);
                    (Some(row.server_version), text, row.deleted)
                }
            };
            conn.execute(
                "INSERT INTO sync_queue (note_id, base_version, base_wcl, base_deleted, queued_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![s.id, base_version, base_wcl, base_deleted, now_ms()],
            )?;
        }
        Ok(())
    }

    /// Every queued change, oldest first.
    pub fn pending(&self) -> Result<Vec<Pending>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT note_id, base_version, base_wcl, base_deleted FROM sync_queue ORDER BY seq",
        )?;
        let rows = stmt.query_map([], pending_row)?;
        rows.collect()
    }

    /// The queued change of note `id`, if there is one.
    pub fn pending_for(&self, id: &str) -> Result<Option<Pending>> {
        self.conn()
            .query_row(
                "SELECT note_id, base_version, base_wcl, base_deleted FROM sync_queue
                 WHERE note_id = ?1",
                [id],
                pending_row,
            )
            .optional()
    }

    /// Take note `id` out of the queue.
    pub fn dequeue(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM sync_queue WHERE note_id = ?1", [id])?;
        Ok(())
    }

    /// Start note `id`'s queued change again from a newer server version, keeping its place.
    pub fn rebase(&self, id: &str, server_version: &str, server_wcl: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE sync_queue SET base_version = ?2, base_wcl = ?3, base_deleted = 0
             WHERE note_id = ?1",
            params![id, server_version, server_wcl],
        )?;
        Ok(())
    }

    /// Where note `id` stands against the server, if the device has ever synced it.
    pub fn synced(&self, id: &str) -> Result<Option<Synced>> {
        synced_row(&self.conn(), id)
    }

    /// Record that note `id`'s local copy matches `server_version`, held locally as
    /// `local_version`, or that the server deleted it.
    pub fn set_synced(
        &self,
        id: &str,
        server_version: &str,
        local_version: Option<&str>,
        deleted: bool,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO sync_notes (note_id, server_version, local_version, deleted)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, server_version, local_version, deleted],
        )?;
        Ok(())
    }

    /// The change-feed cursor: the `next` of the last page applied.
    pub fn cursor(&self) -> Result<i64> {
        let v: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM sync_meta WHERE key = ?1",
                [CURSOR],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub fn set_cursor(&self, cursor: i64) -> Result<()> {
        self.set_meta(CURSOR, &cursor.to_string())
    }

    fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn()
            .query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// The id of the home server's vault the local vault is a copy of, once a handshake or a
    /// pairing recorded it. A local vault synced before ticket 143 has none.
    pub fn server_vault(&self) -> Result<Option<String>> {
        Ok(self.meta(SERVER_VAULT)?.filter(|v| !v.is_empty()))
    }

    pub fn set_server_vault(&self, vault_id: &str) -> Result<()> {
        self.set_meta(SERVER_VAULT, vault_id)
    }

    /// Whether the local vault ever synced with a home server: a feed cursor past the start, or
    /// a note the server answered for. Local writes alone, such as a share made before pairing,
    /// are not: they belong to whichever vault the device joins first.
    pub fn has_synced(&self) -> Result<bool> {
        if self.cursor()? > 0 {
            return Ok(true);
        }
        let n: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM sync_notes", [], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// The last fresh start of this local vault, as the app recorded it, `{ at_ms, previous,
    /// from_vault, to_vault }`.
    pub fn fresh_start(&self) -> Result<Option<serde_json::Value>> {
        Ok(self
            .meta(FRESH_START)?
            .and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn set_fresh_start(&self, record: &serde_json::Value) -> Result<()> {
        self.set_meta(FRESH_START, &record.to_string())
    }

    /// The vault the phone's contact links were last cleaned up for.
    pub fn links_cleaned(&self) -> Result<Option<String>> {
        self.meta(LINKS_CLEANED)
    }

    pub fn set_links_cleaned(&self, vault_id: &str) -> Result<()> {
        self.set_meta(LINKS_CLEANED, vault_id)
    }

    /// Every asset the engine knows of.
    pub fn assets(&self) -> Result<Vec<KnownAsset>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT asset_id, filename, size, on_server, used_ms FROM sync_assets ORDER BY asset_id",
        )?;
        let rows = stmt.query_map([], asset_row)?;
        rows.collect()
    }

    /// Asset `id`, if the engine knows of it.
    pub fn asset(&self, id: &str) -> Result<Option<KnownAsset>> {
        self.conn()
            .query_row(
                "SELECT asset_id, filename, size, on_server, used_ms FROM sync_assets
                 WHERE asset_id = ?1",
                [id],
                asset_row,
            )
            .optional()
    }

    /// Whether the server holds asset `id`, so a local note may name it while its file is not on
    /// the device (D19).
    pub fn asset_on_server(&self, id: &str) -> bool {
        self.conn()
            .query_row(
                "SELECT on_server FROM sync_assets WHERE asset_id = ?1",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    /// Record asset `id`, keeping when it was last used.
    pub fn set_asset(&self, id: &str, filename: &str, size: u64, on_server: bool) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_assets (asset_id, filename, size, on_server, used_ms)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT (asset_id) DO UPDATE SET
                 filename = excluded.filename, size = excluded.size, on_server = excluded.on_server",
            params![id, filename, size as i64, on_server],
        )?;
        Ok(())
    }

    /// Mark asset `id` used now, for the cache's eviction order.
    pub fn touch_asset(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE sync_assets SET used_ms = ?2 WHERE asset_id = ?1",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    /// Record that asset `id` failed to reach the server with `error`, `attempts` times in a row,
    /// to be tried again from `next_ms`.
    pub fn set_asset_failure(&self, f: &AssetFailure) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO sync_asset_failures
                 (asset_id, filename, size, error, attempts, last_ms, next_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                f.id,
                f.filename,
                f.size as i64,
                f.error,
                f.attempts,
                f.last_ms,
                f.next_ms
            ],
        )?;
        Ok(())
    }

    /// Forget asset `id`'s failures: it reached the server, or is gone from the device.
    pub fn clear_asset_failure(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM sync_asset_failures WHERE asset_id = ?1", [id])?;
        Ok(())
    }

    /// Asset `id`'s last failure, if it has not reached the server since.
    pub fn asset_failure(&self, id: &str) -> Result<Option<AssetFailure>> {
        self.conn()
            .query_row(
                "SELECT asset_id, filename, size, error, attempts, last_ms, next_ms
                 FROM sync_asset_failures WHERE asset_id = ?1",
                [id],
                failure_row,
            )
            .optional()
    }

    /// Every asset that failed to reach the server, the one failing longest first.
    pub fn asset_failures(&self) -> Result<Vec<AssetFailure>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT asset_id, filename, size, error, attempts, last_ms, next_ms
             FROM sync_asset_failures ORDER BY attempts DESC, asset_id",
        )?;
        let rows = stmt.query_map([], failure_row)?;
        rows.collect()
    }

    /// Let the next round try every failed asset again at once: the "Sync now" button.
    pub fn retry_assets_now(&self) -> Result<()> {
        self.conn()
            .execute("UPDATE sync_asset_failures SET next_ms = 0", [])?;
        Ok(())
    }

    /// Replace the notes the last round held back.
    pub fn set_held(&self, held: &[Held]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM sync_held", [])?;
        for h in held {
            tx.execute(
                "INSERT OR REPLACE INTO sync_held (note_id, asset_id, reason) VALUES (?1, ?2, ?3)",
                params![h.note_id, h.asset_id.as_deref().unwrap_or(""), h.reason],
            )?;
        }
        tx.commit()
    }

    /// The queued notes the last round held back, each with the asset it waits for, if any.
    pub fn held(&self) -> Result<Vec<Held>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT h.note_id, h.asset_id, h.reason FROM sync_held h
             JOIN sync_queue q ON q.note_id = h.note_id ORDER BY q.seq",
        )?;
        let rows = stmt.query_map([], |r| {
            let asset: String = r.get(1)?;
            Ok(Held {
                note_id: r.get(0)?,
                asset_id: (!asset.is_empty()).then_some(asset),
                reason: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Record that note `id` was opened on the device.
    pub fn opened(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO sync_recent (note_id, opened_ms) VALUES (?1, ?2)",
            params![id, now_ms()],
        )?;
        Ok(())
    }

    /// The `limit` notes opened most recently, newest first.
    pub fn recent(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT note_id FROM sync_recent ORDER BY opened_ms DESC, note_id LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| r.get(0))?;
        rows.collect()
    }
}

fn pending_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Pending> {
    Ok(Pending {
        note_id: r.get(0)?,
        base_version: r.get(1)?,
        base_wcl: r.get(2)?,
        base_deleted: r.get(3)?,
    })
}

fn asset_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<KnownAsset> {
    Ok(KnownAsset {
        id: r.get(0)?,
        filename: r.get(1)?,
        size: r.get::<_, i64>(2)? as u64,
        on_server: r.get(3)?,
        used_ms: r.get(4)?,
    })
}

fn failure_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AssetFailure> {
    Ok(AssetFailure {
        id: r.get(0)?,
        filename: r.get(1)?,
        size: r.get::<_, i64>(2)? as u64,
        error: r.get(3)?,
        attempts: r.get(4)?,
        last_ms: r.get(5)?,
        next_ms: r.get(6)?,
    })
}

fn synced_row(conn: &Connection, id: &str) -> Result<Option<Synced>> {
    conn.query_row(
        "SELECT server_version, local_version, deleted FROM sync_notes WHERE note_id = ?1",
        [id],
        |r| {
            Ok(Synced {
                server_version: r.get(0)?,
                local_version: r.get(1)?,
                deleted: r.get(2)?,
            })
        },
    )
    .optional()
}
