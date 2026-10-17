//! The shipped defaults written into the vault, and the one-time upgrade steps applied to it.
//!
//! For each shipped file the server wrote, such as `scripts/device_calls.ws`, `state.db` keeps
//! the content hash of what it wrote, so a later start can tell a file the user left alone from
//! one the user changed. It also keeps the hash of the shipped version the user was last told
//! about, so a changed file earns one notification per shipped version rather than one per start.
//! The `writer` crate decides what to write; this only remembers.

use rusqlite::{OptionalExtension, params};

use crate::{Result, State};

/// What `state.db` knows about one shipped file in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedRecord {
    /// The SHA-256 hex of the content last written, empty when the server never wrote it.
    pub hash: String,
    /// The hash of the shipped version the user was last notified about, if any.
    pub notified: Option<String>,
}

impl State {
    /// What is recorded about shipped file `path`, or `None` when nothing is.
    pub fn shipped_file(&self, path: &str) -> Result<Option<ShippedRecord>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT hash, notified FROM shipped_files WHERE path = ?1",
                [path],
                |r| {
                    Ok(ShippedRecord {
                        hash: r.get(0)?,
                        notified: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Record that `path` now holds the shipped content whose hash is `hash`.
    pub fn record_shipped(&self, path: &str, hash: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO shipped_files (path, hash, updated_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT (path) DO UPDATE SET hash = excluded.hash, updated_ms = excluded.updated_ms",
            params![path, hash, self.now()],
        )?;
        Ok(())
    }

    /// Record that the user was told shipped version `hash` of `path` exists.
    pub fn record_shipped_notice(&self, path: &str, hash: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO shipped_files (path, hash, notified, updated_ms) VALUES (?1, '', ?2, ?3)
             ON CONFLICT (path) DO UPDATE SET notified = excluded.notified,
                 updated_ms = excluded.updated_ms",
            params![path, hash, self.now()],
        )?;
        Ok(())
    }

    /// The id of every upgrade step applied to the vault, in id order.
    pub fn applied_upgrades(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id FROM upgrades ORDER BY id")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    /// Record that upgrade step `id` has been applied. Recording it again changes nothing.
    pub fn record_upgrade(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO upgrades (id, applied_ms) VALUES (?1, ?2)",
            params![id, self.now()],
        )?;
        Ok(())
    }
}
