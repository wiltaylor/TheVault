//! Note version history, trash and history pruning (D13, D25).

use rusqlite::{OptionalExtension, params};

use crate::{Error, Result, State, random_bytes, to_hex};

/// Who made a change (D13, D30).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Author {
    /// A browser session.
    Session(String),
    /// A named API token.
    Token(String),
    /// A named wscript handler.
    Handler(String),
}

impl Author {
    fn parts(&self) -> (&'static str, &str) {
        match self {
            Author::Session(n) => ("session", n),
            Author::Token(n) => ("token", n),
            Author::Handler(n) => ("handler", n),
        }
    }

    fn from_parts(kind: &str, name: String) -> Result<Self> {
        match kind {
            "session" => Ok(Author::Session(name)),
            "token" => Ok(Author::Token(name)),
            "handler" => Ok(Author::Handler(name)),
            other => Err(Error::Corrupt(format!("unknown author kind {other:?}"))),
        }
    }
}

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

type VersionRow = (Version, String, String);

fn version_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VersionRow> {
    Ok((
        Version {
            note_id: row.get(0)?,
            note_type: row.get(1)?,
            token: row.get(2)?,
            author: Author::Session(String::new()),
            at_ms: row.get(5)?,
            content: row.get(6)?,
        },
        row.get(3)?,
        row.get(4)?,
    ))
}

fn finish_version((mut version, kind, name): VersionRow) -> Result<Version> {
    version.author = Author::from_parts(&kind, name)?;
    Ok(version)
}

impl State {
    /// Records `content` as a new version of the note and makes it the current one. A note
    /// in the trash leaves it, since it exists again.
    pub fn record_version(
        &self,
        note_id: &str,
        note_type: &str,
        author: &Author,
        content: &str,
    ) -> Result<Version> {
        let mut raw = [0u8; 16];
        random_bytes(&mut raw);
        let token = to_hex(&raw);
        let at_ms = self.now();
        let (kind, name) = author.parts();

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO versions (note_id, note_type, token, author_kind, author_name, at_ms, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![note_id, note_type, token, kind, name, at_ms, content],
        )?;
        tx.execute(
            "INSERT INTO heads (note_id, token) VALUES (?1, ?2)
             ON CONFLICT (note_id) DO UPDATE SET token = excluded.token",
            params![note_id, token],
        )?;
        tx.execute("DELETE FROM trash WHERE note_id = ?1", [note_id])?;
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
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
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

    /// Removes old versions under `policy` and returns how many were removed.
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
                 )",
                [keep],
            )?;
        }
        if let Some(age) = policy.max_age_ms {
            let cutoff = self.now() - age;
            removed += tx.execute(
                "DELETE FROM versions
                 WHERE at_ms < ?1
                   AND seq NOT IN (SELECT MAX(seq) FROM versions GROUP BY note_id)",
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
    entry.deleted_by = Author::from_parts(&kind, name)?;
    Ok(entry)
}
