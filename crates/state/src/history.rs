//! Note version history, trash and history pruning (D13, D25).

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::changes::{ChangeKey, journal};
use crate::{Error, Result, State, random_bytes, to_hex};

pub use vocab::Author;

/// One recorded version of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub note_id: String,
    pub note_type: String,
    /// Opaque to clients; compared for equality only (D12).
    pub token: String,
    pub author: Author,
    pub at_ms: i64,
    pub content: String,
}

/// Browser saves of one note this close together merge into one version.
pub const MERGE_GAP_MS: i64 = 2 * 60 * 1000;

/// A run of merged browser saves stops merging once it is this old, so a long session still
/// leaves a version every so often.
pub const MERGE_SPAN_MS: i64 = 10 * 60 * 1000;

/// Compressed snapshot input and decoded limit. Larger snapshots use legacy text.
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const SNAPSHOT_MAGIC: &[u8; 4] = b"VHS\x01";
const SNAPSHOT_HEADER: usize = 44;

fn encode_snapshot(content: &[u8]) -> Result<Vec<u8>> {
    if content.len() > MAX_SNAPSHOT_BYTES {
        return Err(Error::Corrupt(
            "history snapshot exceeds 64 MiB limit".into(),
        ));
    }
    let mut blob = Vec::with_capacity(SNAPSHOT_HEADER);
    blob.extend_from_slice(SNAPSHOT_MAGIC);
    blob.extend_from_slice(&(content.len() as u64).to_le_bytes());
    blob.extend_from_slice(&Sha256::digest(content));
    blob.extend_from_slice(&lz4_flex::block::compress(content));
    Ok(blob)
}

fn decode_snapshot(blob: &[u8]) -> Result<Vec<u8>> {
    let corrupt = || Error::Corrupt("invalid or corrupt history snapshot".into());
    if blob.len() < SNAPSHOT_HEADER || &blob[..4] != SNAPSHOT_MAGIC {
        return Err(corrupt());
    }
    let size = u64::from_le_bytes(blob[4..12].try_into().map_err(|_| corrupt())?);
    if size > MAX_SNAPSHOT_BYTES as u64
        || blob.len()
            > lz4_flex::block::get_maximum_output_size(MAX_SNAPSHOT_BYTES) + SNAPSHOT_HEADER
    {
        return Err(Error::Corrupt(
            "history snapshot exceeds 64 MiB limit".into(),
        ));
    }
    // Never trust the decoder's size prefix. A fixed output slice bounds allocation and
    // work is bounded by both the input and output lengths of one LZ4 block.
    let mut content = vec![0; size as usize];
    let written = lz4_flex::block::decompress_into(&blob[SNAPSHOT_HEADER..], &mut content)
        .map_err(|_| corrupt())?;
    if written != content.len() || Sha256::digest(&content)[..] != blob[12..44] {
        return Err(corrupt());
    }
    Ok(content)
}

pub(crate) fn snapshot_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    use rusqlite::types::{Type, ValueRef};
    let decode = || -> Result<String> {
        match row.get_ref(index)? {
            ValueRef::Text(text) => String::from_utf8(text.to_vec())
                .map_err(|_| Error::Corrupt("invalid history UTF-8".into())),
            ValueRef::Blob(blob) => String::from_utf8(decode_snapshot(blob)?)
                .map_err(|_| Error::Corrupt("invalid history UTF-8".into())),
            _ => Err(Error::Corrupt("invalid history content type".into())),
        }
    };
    decode()
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(index, Type::Blob, Box::new(err)))
}

/// A deleted note waiting in the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub note_id: String,
    pub note_type: String,
    /// The note's last content before it was deleted.
    pub content: String,
    /// The version token the note had when it was deleted.
    pub version: String,
    pub deleted_by: Author,
    pub deleted_ms: i64,
}

/// Which old versions [`State::prune`] removes. Both limits off, the default, keeps
/// everything (D25). A note's current version is never removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePolicy {
    /// Keep at most this many versions per note, newest first.
    pub keep_versions: Option<u32>,
    /// Remove versions older than this many milliseconds.
    pub max_age_ms: Option<i64>,
}

const VERSION_COLUMNS: &str = "note_id, note_type, token, author_kind, author_name, at_ms, content";

/// Rows and UTF-8 content bytes eligible for thread-history pruning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHistoryPrune {
    pub note_id: String,
    pub rows: u64,
    pub bytes: u64,
}

// Device bases are additional to the newest N rows, as in State::prune (D29).
const THREAD_PRUNE_ROWS: &str = "SELECT seq FROM (
    SELECT seq, token, ROW_NUMBER() OVER (PARTITION BY note_id ORDER BY seq DESC) AS n
    FROM versions WHERE note_type = 'agent_thread' AND (?2 IS NULL OR note_id = ?2)
) WHERE n > ?1 AND token NOT IN (SELECT version FROM device_bases)
  AND token NOT IN (SELECT token FROM heads)";

type VersionRow = (Version, String, String);

fn version_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VersionRow> {
    Ok((
        Version {
            note_id: row.get(0)?,
            note_type: row.get(1)?,
            token: row.get(2)?,
            author: Author::Session(String::new()),
            at_ms: row.get(5)?,
            content: snapshot_column(row, 6)?,
        },
        row.get(3)?,
        row.get(4)?,
    ))
}

fn finish_version((mut version, kind, name): VersionRow) -> Result<Version> {
    version.author = author_from_parts(&kind, name)?;
    Ok(version)
}

impl State {
    /// Records `content` as a new version of the note and makes it the current one. A note
    /// in the trash leaves it, since it exists again.
    /// Record `content` as the note's new version. A browser save merges into the current version
    /// instead of adding one when that version is the same session's, saved within
    /// [`MERGE_GAP_MS`], and its run of merged saves began within [`MERGE_SPAN_MS`]. Autosave
    /// writes every few seconds while someone types, and this keeps that from filling the history.
    /// A merged save still gets a new token, so a client holding the old one gets a conflict
    /// (D12). Saves by tokens and handlers always add a version, so each one stays visible: a
    /// device's token is one, so its writes never merge (D29). Nor does a save merge into a version
    /// a device holds as its base, since merging rewrites that version.
    pub fn record_version(
        &self,
        note_id: &str,
        note_type: &str,
        author: &Author,
        content: &str,
    ) -> Result<Version> {
        self.record(note_id, note_type, author, content, true)
    }

    /// [`State::record_version`] that always adds a version, never merging into the current one:
    /// for a change that should stay visible in the history whoever made it, such as archiving.
    pub fn record_distinct_version(
        &self,
        note_id: &str,
        note_type: &str,
        author: &Author,
        content: &str,
    ) -> Result<Version> {
        self.record(note_id, note_type, author, content, false)
    }

    fn record(
        &self,
        note_id: &str,
        note_type: &str,
        author: &Author,
        content: &str,
        merge: bool,
    ) -> Result<Version> {
        let encoded = if content.len() > MAX_SNAPSHOT_BYTES {
            rusqlite::types::Value::Text(content.to_owned())
        } else {
            rusqlite::types::Value::Blob(encode_snapshot(content.as_bytes())?)
        };
        let mut raw = [0u8; 16];
        random_bytes(&mut raw);
        let token = to_hex(&raw);
        let at_ms = self.now();
        let (kind, name) = author.parts();

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let merge_into: Option<i64> = if merge && matches!(author, Author::Session(_)) {
            tx.query_row(
                "SELECT v.seq FROM heads h JOIN versions v ON v.token = h.token
                 WHERE h.note_id = ?1 AND v.note_type = ?2 AND v.author_kind = ?3
                   AND v.author_name = ?4 AND ?5 - v.at_ms <= ?6
                   AND ?5 - COALESCE(v.started_ms, v.at_ms) <= ?7
                   AND NOT EXISTS (SELECT 1 FROM device_bases b WHERE b.version = v.token)",
                params![
                    note_id,
                    note_type,
                    kind,
                    name,
                    at_ms,
                    MERGE_GAP_MS,
                    MERGE_SPAN_MS
                ],
                |r| r.get(0),
            )
            .optional()?
        } else {
            None
        };
        match merge_into {
            Some(seq) => {
                tx.execute(
                    "UPDATE versions
                     SET token = ?1, at_ms = ?2, content = ?3, content_bytes = ?5,
                         started_ms = COALESCE(started_ms, at_ms)
                     WHERE seq = ?4",
                    params![token, at_ms, encoded, seq, content.len() as i64],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO versions
                       (note_id, note_type, token, author_kind, author_name, at_ms, content, started_ms, content_bytes)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?8)",
                    params![note_id, note_type, token, kind, name, at_ms, encoded, content.len() as i64],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO heads (note_id, token) VALUES (?1, ?2)
             ON CONFLICT (note_id) DO UPDATE SET token = excluded.token",
            params![note_id, token],
        )?;
        tx.execute("DELETE FROM trash WHERE note_id = ?1", [note_id])?;
        journal(&tx, &ChangeKey::Note(note_id.to_owned()), true, at_ms)?;
        if note_type == "agent_thread" {
            tx.execute(
                &format!("DELETE FROM versions WHERE seq IN ({THREAD_PRUNE_ROWS})"),
                params![3, note_id],
            )?;
        }
        tx.commit()?;

        Ok(Version {
            note_id: note_id.to_owned(),
            note_type: note_type.to_owned(),
            token,
            author: author.clone(),
            at_ms,
            content: content.to_owned(),
        })
    }

    /// The current version token of a live note, or `None` when it has none or is trashed.
    pub fn current_version(&self, note_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT token FROM heads WHERE note_id = ?1",
                [note_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The current version of a live note, or `None` when it has none or is trashed.
    pub fn head(&self, note_id: &str) -> Result<Option<Version>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                &format!(
                    "SELECT {VERSION_COLUMNS} FROM versions
                     WHERE token = (SELECT token FROM heads WHERE note_id = ?1)"
                ),
                [note_id],
                version_row,
            )
            .optional()?;
        row.map(finish_version).transpose()
    }

    /// The id of every note with a current version, sorted.
    pub fn live_notes(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT note_id FROM heads ORDER BY note_id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every recorded version of the note, oldest first.
    pub fn history(&self, note_id: &str) -> Result<Vec<Version>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {VERSION_COLUMNS} FROM versions WHERE note_id = ?1 ORDER BY seq"
        ))?;
        let rows = stmt.query_map([note_id], version_row)?;
        rows.map(|r| finish_version(r?)).collect()
    }

    /// History needed by a call scan. Keep raw-id matches even if the wall clock
    /// stepped backwards after issuance. The timestamp bound only saves work.
    pub fn agent_call_history(&self, note_id: &str) -> Result<Vec<Version>> {
        let conn = self.conn();
        let cutoff: Option<i64> = conn.query_row(
            "SELECT MIN(expires_ms - 86400000) FROM agent_call_ids WHERE note=?1 AND used=0",
            [note_id],
            |r| r.get(0),
        )?;
        let mut calls_stmt =
            conn.prepare("SELECT call FROM agent_call_ids WHERE note=?1 AND used=0")?;
        let calls = calls_stmt
            .query_map([note_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {VERSION_COLUMNS} FROM versions WHERE note_id = ?1 AND (
                at_ms >= (SELECT MIN(expires_ms - 86400000) FROM agent_call_ids
                          WHERE note = ?1 AND used = 0)
                OR EXISTS (SELECT 1 FROM agent_call_ids
                           WHERE note = ?1 AND used = 0 AND
                             (typeof(content) = 'blob' OR instr(content, call) > 0))
             ) ORDER BY seq"
        ))?;
        let rows = stmt.query_map([note_id], version_row)?;
        let mut versions = Vec::new();
        for row in rows {
            let version = finish_version(row?)?;
            if cutoff.is_some_and(|at| version.at_ms >= at)
                || calls.iter().any(|call| version.content.contains(call))
            {
                versions.push(version);
            }
        }
        Ok(versions)
    }

    /// One version of the note by its token.
    pub fn version(&self, note_id: &str, token: &str) -> Result<Option<Version>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                &format!(
                    "SELECT {VERSION_COLUMNS} FROM versions WHERE note_id = ?1 AND token = ?2"
                ),
                params![note_id, token],
                version_row,
            )
            .optional()?;
        row.map(finish_version).transpose()
    }

    /// Moves a deleted note to the trash, keeping its last content.
    pub fn trash_note(&self, note_id: &str, author: &Author) -> Result<TrashEntry> {
        let deleted_ms = self.now();
        let (kind, name) = author.parts();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let last: Option<(String, String, String)> = tx
            .query_row(
                "SELECT note_type, token, content FROM versions
                 WHERE note_id = ?1 ORDER BY seq DESC LIMIT 1",
                [note_id],
                |r| Ok((r.get(0)?, r.get(1)?, snapshot_column(r, 2)?)),
            )
            .optional()?;
        let Some((note_type, version, content)) = last else {
            return Err(Error::UnknownNote(note_id.to_owned()));
        };
        tx.execute(
            "INSERT OR REPLACE INTO trash
             (note_id, note_type, content, version, author_kind, author_name, deleted_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![note_id, note_type, content, version, kind, name, deleted_ms],
        )?;
        tx.execute("DELETE FROM heads WHERE note_id = ?1", [note_id])?;
        journal(&tx, &ChangeKey::Note(note_id.to_owned()), false, deleted_ms)?;
        tx.commit()?;
        Ok(TrashEntry {
            note_id: note_id.to_owned(),
            note_type,
            content,
            version,
            deleted_by: author.clone(),
            deleted_ms,
        })
    }

    /// Every note in the trash, most recently deleted first.
    pub fn list_trash(&self) -> Result<Vec<TrashEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT note_id, note_type, content, version, author_kind, author_name, deleted_ms
             FROM trash ORDER BY deleted_ms DESC, note_id",
        )?;
        let rows = stmt.query_map([], trash_row)?;
        rows.map(|r| finish_trash(r?)).collect()
    }

    /// Note `note_id` as it sits in the trash, or `None` when it is not there.
    pub fn trash_entry(&self, note_id: &str) -> Result<Option<TrashEntry>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT note_id, note_type, content, version, author_kind, author_name, deleted_ms
                 FROM trash WHERE note_id = ?1",
                [note_id],
                trash_row,
            )
            .optional()?;
        row.map(finish_trash).transpose()
    }

    /// Whether note `note_id` is in the trash.
    pub fn in_trash(&self, note_id: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM trash WHERE note_id = ?1)",
            [note_id],
            |r| r.get(0),
        )?)
    }

    /// Removes note `note_id` from the trash for good, if it is there.
    pub fn purge_from_trash(&self, note_id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM trash WHERE note_id = ?1", [note_id])?;
        Ok(())
    }

    /// Removes note `note_id` from the trash for good, with its whole history and every device's
    /// base of it (ai D21). The change feed keeps the note as deleted, so a device still learns
    /// of the delete. Returns false, changing nothing, when the note is not in the trash.
    pub fn purge_note(&self, note_id: &str) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if tx.execute("DELETE FROM trash WHERE note_id = ?1", [note_id])? == 0 {
            return Ok(false);
        }
        tx.execute("DELETE FROM versions WHERE note_id = ?1", [note_id])?;
        tx.execute("DELETE FROM device_bases WHERE note_id = ?1", [note_id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Takes a note out of the trash and returns its last content. The caller writes the
    /// file back and records a new version.
    pub fn restore_from_trash(&self, note_id: &str) -> Result<TrashEntry> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let row = tx
            .query_row(
                "SELECT note_id, note_type, content, version, author_kind, author_name, deleted_ms
                 FROM trash WHERE note_id = ?1",
                [note_id],
                trash_row,
            )
            .optional()?;
        let Some(row) = row else {
            return Err(Error::NotInTrash(note_id.to_owned()));
        };
        tx.execute("DELETE FROM trash WHERE note_id = ?1", [note_id])?;
        tx.commit()?;
        finish_trash(row)
    }

    /// Preview or apply thread-only pruning. Counts refer to content bytes, not SQLite file
    /// size. Preview and deletion use one transaction. Device bases and heads are protected.
    pub fn prune_thread_history(&self, keep: u32, apply: bool) -> Result<Vec<ThreadHistoryPrune>> {
        if keep == 0 {
            return Err(Error::InvalidPrunePolicy("keep must be at least 1".into()));
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let report = {
            let mut stmt = tx.prepare(&format!(
                "SELECT note_id, COUNT(*), SUM(COALESCE(content_bytes, length(CAST(content AS BLOB)))) FROM versions
                 WHERE seq IN ({THREAD_PRUNE_ROWS}) GROUP BY note_id ORDER BY note_id"
            ))?;
            stmt.query_map(params![keep, Option::<String>::None], |r| {
                Ok(ThreadHistoryPrune {
                    note_id: r.get(0)?,
                    rows: r.get(1)?,
                    bytes: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if apply {
            tx.execute(
                &format!("DELETE FROM versions WHERE seq IN ({THREAD_PRUNE_ROWS})"),
                params![keep, Option::<String>::None],
            )?;
        }
        tx.commit()?;
        Ok(report)
    }

    /// Removes old versions under `policy` and returns how many were removed. A version a device
    /// holds as its base is never removed (D29).
    pub fn prune(&self, policy: &PrunePolicy) -> Result<usize> {
        if policy.keep_versions == Some(0) {
            return Err(Error::InvalidPrunePolicy(
                "keep_versions must be at least 1".into(),
            ));
        }
        if policy.max_age_ms.is_some_and(|age| age < 0) {
            return Err(Error::InvalidPrunePolicy(
                "max_age_ms must not be negative".into(),
            ));
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut removed = 0;
        if let Some(keep) = policy.keep_versions {
            removed += tx.execute(
                "DELETE FROM versions WHERE seq IN (
                     SELECT seq FROM (
                         SELECT seq, ROW_NUMBER() OVER (PARTITION BY note_id ORDER BY seq DESC) AS n
                         FROM versions
                     ) WHERE n > ?1
                 )
                 AND token NOT IN (SELECT version FROM device_bases)",
                [keep],
            )?;
        }
        if let Some(age) = policy.max_age_ms {
            let cutoff = self.now() - age;
            removed += tx.execute(
                "DELETE FROM versions
                 WHERE at_ms < ?1
                   AND seq NOT IN (SELECT MAX(seq) FROM versions GROUP BY note_id)
                   AND token NOT IN (SELECT version FROM device_bases)",
                [cutoff],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }
}

type TrashRow = (TrashEntry, String, String);

fn trash_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrashRow> {
    Ok((
        TrashEntry {
            note_id: row.get(0)?,
            note_type: row.get(1)?,
            content: row.get(2)?,
            version: row.get(3)?,
            deleted_by: Author::Session(String::new()),
            deleted_ms: row.get(6)?,
        },
        row.get(4)?,
        row.get(5)?,
    ))
}

fn finish_trash((mut entry, kind, name): TrashRow) -> Result<TrashEntry> {
    entry.deleted_by = author_from_parts(&kind, name)?;
    Ok(entry)
}

fn author_from_parts(kind: &str, name: String) -> Result<Author> {
    Author::from_parts(kind, name)
        .ok_or_else(|| Error::Corrupt(format!("unknown author kind {kind:?}")))
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn byte_exact_text_binary_empty_and_large() {
        for bytes in [
            Vec::new(),
            "text\r\n雪\0é".as_bytes().to_vec(),
            (0..=255).collect(),
            vec![42; 4 * 1024 * 1024],
        ] {
            assert_eq!(
                decode_snapshot(&encode_snapshot(&bytes).unwrap()).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn bounded_corrupt_unknown_and_bomb() {
        let blob = encode_snapshot(&vec![0; 1024 * 1024]).unwrap();
        let mut bomb = blob.clone();
        bomb[4..12].copy_from_slice(&((MAX_SNAPSHOT_BYTES as u64) + 1).to_le_bytes());
        assert!(
            decode_snapshot(&bomb)
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
        // A tiny declared output must not let an expanding block allocate more.
        bomb[4..12].copy_from_slice(&1u64.to_le_bytes());
        assert!(decode_snapshot(&bomb).is_err());
        for n in [0, 1, 4, 12, 43, blob.len() - 1] {
            assert!(decode_snapshot(&blob[..n]).is_err());
        }
        let mut corrupt = blob.clone();
        corrupt[12] ^= 1;
        assert!(decode_snapshot(&corrupt).is_err());
        corrupt[3] = 2;
        assert!(decode_snapshot(&corrupt).is_err());
        assert!(encode_snapshot(&vec![0; MAX_SNAPSHOT_BYTES + 1]).is_err());
    }
}
