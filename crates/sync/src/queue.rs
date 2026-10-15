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
//! - `sync_recent`: notes opened on the device, for the on-demand asset policy.
//! - `sync_meta`: the change-feed cursor.

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

/// The device's sync state. Opened on the local `state.db`.
pub struct Queue {
    conn: Mutex<Connection>,
}

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

fn now_ms() -> i64 {
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
        self.conn().execute(
            "INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, ?2)",
            params![CURSOR, cursor.to_string()],
        )?;
        Ok(())
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
