//! The change feed devices sync from, and the base versions they still hold (D20, D29).
//!
//! `changes` holds one row per note and per synced vault file, and each change replaces the row
//! with a new `seq`. Reading every row past a cursor in `seq` order gives each key once, at its
//! latest change, so a device that pages through the feed sees every key that changed since its
//! cursor and nothing twice. A row says only whether the key exists; the feed reads its content
//! as it is now.
//!
//! A note's row is written in the same transaction as its version or its move to the trash. The
//! writer journals the other files after each commit and reconciles them on open.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, params};

use crate::{Error, Result, State};

/// What one feed entry is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChangeKey {
    /// A note, by id. Its type can change, so its path is not its key.
    Note(String),
    /// Any other vault file, by path, such as `types/memo.wcl`.
    File(String),
}

impl ChangeKey {
    fn parts(&self) -> (&'static str, &str) {
        match self {
            ChangeKey::Note(id) => ("note", id),
            ChangeKey::File(path) => ("file", path),
        }
    }
}

/// One key's latest change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journaled {
    pub seq: i64,
    pub key: ChangeKey,
    /// Whether the note is live, or the file exists, after the change.
    pub present: bool,
    pub at_ms: i64,
}

/// One page of the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePage {
    pub changes: Vec<Journaled>,
    /// The cursor for the next page: the last entry's `seq`, or the cursor asked for when the
    /// page is empty.
    pub next: i64,
    /// The newest `seq` in the feed. A device is up to date once `next` reaches it.
    pub latest: i64,
}

/// Record that `key` changed. Replacing the row gives it a new, larger `seq`.
pub(crate) fn journal(conn: &Connection, key: &ChangeKey, present: bool, at_ms: i64) -> Result<()> {
    let (kind, name) = key.parts();
    conn.execute(
        "INSERT OR REPLACE INTO changes (kind, name, present, at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![kind, name, present, at_ms],
    )?;
    Ok(())
}

impl State {
    /// Record changes to vault files other than notes: `(path, exists)` for each.
    pub fn journal_files(&self, files: &[(String, bool)]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let at_ms = self.now();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (path, present) in files {
            journal(&tx, &ChangeKey::File(path.clone()), *present, at_ms)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Bring the file rows of the feed up to `on_disk`, every synced file that exists now: a file
    /// the feed does not show as existing is journaled as written, and one it shows that is gone
    /// as deleted. Covers files written before the feed existed, files added outside the server,
    /// and a crash between a commit and its journal entry. Returns how many rows changed.
    pub fn reconcile_files(&self, on_disk: &[String]) -> Result<usize> {
        let at_ms = self.now();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let journaled: BTreeMap<String, bool> = {
            let mut stmt = tx.prepare("SELECT name, present FROM changes WHERE kind = 'file'")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let disk: HashSet<&str> = on_disk.iter().map(String::as_str).collect();
        let mut changed = 0;
        for path in on_disk {
            if journaled.get(path) != Some(&true) {
                journal(&tx, &ChangeKey::File(path.clone()), true, at_ms)?;
                changed += 1;
            }
        }
        for (path, present) in &journaled {
            if *present && !disk.contains(path.as_str()) {
                journal(&tx, &ChangeKey::File(path.clone()), false, at_ms)?;
                changed += 1;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Up to `limit` keys whose latest change comes after `since`, oldest first.
    pub fn changes_since(&self, since: i64, limit: usize) -> Result<ChangePage> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT seq, kind, name, present, at_ms FROM changes
             WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![since, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut changes = Vec::new();
        for row in rows {
            let (seq, kind, name, present, at_ms) = row?;
            let key = match kind.as_str() {
                "note" => ChangeKey::Note(name),
                "file" => ChangeKey::File(name),
                other => return Err(Error::Corrupt(format!("unknown change kind {other:?}"))),
            };
            changes.push(Journaled {
                seq,
                key,
                present,
                at_ms,
            });
        }
        let latest: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM changes", [], |r| {
            r.get(0)
        })?;
        let next = changes.last().map_or(since, |c| c.seq);
        Ok(ChangePage {
            changes,
            next,
            latest,
        })
    }

    /// Replace the base versions device `device_id` holds with `bases`, `(note id, version)`
    /// pairs: for each note, the oldest version a write the device has queued is based on.
    /// Pruning keeps these versions, and a browser save does not merge into one (D29). Each
    /// version must still exist.
    pub fn set_device_bases(&self, device_id: &str, bases: &[(String, String)]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let known: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?1)",
            [device_id],
            |r| r.get(0),
        )?;
        if !known {
            return Err(Error::UnknownDevice(device_id.to_owned()));
        }
        tx.execute("DELETE FROM device_bases WHERE device_id = ?1", [device_id])?;
        for (note, version) in bases {
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM versions WHERE note_id = ?1 AND token = ?2)",
                params![note, version],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(Error::UnknownVersion {
                    note: note.clone(),
                    version: version.clone(),
                });
            }
            tx.execute(
                "INSERT INTO device_bases (device_id, note_id, version) VALUES (?1, ?2, ?3)",
                params![device_id, note, version],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The base versions device `device_id` holds, `(note id, version)`, by note id.
    pub fn device_bases(&self, device_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT note_id, version FROM device_bases WHERE device_id = ?1 ORDER BY note_id",
        )?;
        let rows = stmt.query_map([device_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
