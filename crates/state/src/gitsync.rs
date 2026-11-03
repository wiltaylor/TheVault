//! What git sync knows of each note it shares with another instance (git-sync D8, D10, D17,
//! D27).
//!
//! One row per note and peer instance. Its `base` is the published bytes last taken from or sent
//! to that peer: the merge base when both sides changed the note, and the version an imported
//! note's local changes are worked out against. A row whose note is in the trash or gone is a
//! tombstone: the note is not imported again. Disconnecting clears the table.

use rusqlite::{OptionalExtension, params};

use crate::{Result, State};

/// One note git sync shares with one peer instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitsyncNote {
    pub note_id: String,
    /// The vault id of the peer instance.
    pub peer_vault_id: String,
    /// The vault id of the instance that created the note.
    pub origin: String,
    /// The published bytes last taken from or sent to the peer.
    pub base: Vec<u8>,
    /// The hash of `base`, as the manifest records it.
    pub base_hash: String,
    /// Whether git sync archived the note because it left the peer's published set (D3, D26).
    pub archived_by_sync: bool,
}

const COLUMNS: &str = "note_id, peer_vault_id, origin, base, base_hash, archived_by_sync";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<GitsyncNote> {
    Ok(GitsyncNote {
        note_id: r.get(0)?,
        peer_vault_id: r.get(1)?,
        origin: r.get(2)?,
        base: r.get(3)?,
        base_hash: r.get(4)?,
        archived_by_sync: r.get(5)?,
    })
}

impl State {
    /// The row for note `note_id` and peer `peer_vault_id`, or `None` when there is none.
    pub fn gitsync_get(&self, note_id: &str, peer_vault_id: &str) -> Result<Option<GitsyncNote>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM gitsync_notes WHERE note_id = ?1 AND peer_vault_id = ?2"
                ),
                [note_id, peer_vault_id],
                row,
            )
            .optional()?)
    }

    /// Store `note`, replacing the row for its note and peer.
    pub fn gitsync_put(&self, note: &GitsyncNote) -> Result<()> {
        self.conn().execute(
            &format!(
                "INSERT OR REPLACE INTO gitsync_notes ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
            ),
            params![
                note.note_id,
                note.peer_vault_id,
                note.origin,
                note.base,
                note.base_hash,
                note.archived_by_sync,
            ],
        )?;
        Ok(())
    }

    /// Every row for peer `peer_vault_id`, in note id order.
    pub fn gitsync_rows_for_peer(&self, peer_vault_id: &str) -> Result<Vec<GitsyncNote>> {
        self.gitsync_rows("peer_vault_id = ?1 ORDER BY note_id", peer_vault_id)
    }

    /// Every row for note `note_id`, one per peer, in peer order.
    pub fn gitsync_rows_for_note(&self, note_id: &str) -> Result<Vec<GitsyncNote>> {
        self.gitsync_rows("note_id = ?1 ORDER BY peer_vault_id", note_id)
    }

    /// Every row of a note another instance created, `origin` not `vault_id`: the notes this
    /// vault imported. In note id order, then peer order.
    pub fn gitsync_imported(&self, vault_id: &str) -> Result<Vec<GitsyncNote>> {
        self.gitsync_rows("origin != ?1 ORDER BY note_id, peer_vault_id", vault_id)
    }

    fn gitsync_rows(&self, filter: &str, value: &str) -> Result<Vec<GitsyncNote>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM gitsync_notes WHERE {filter}"
        ))?;
        let rows = stmt
            .query_map([value], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Set whether git sync archived note `note_id` for peer `peer_vault_id`. Returns whether
    /// there was a row to change.
    pub fn gitsync_set_archived_by_sync(
        &self,
        note_id: &str,
        peer_vault_id: &str,
        archived: bool,
    ) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE gitsync_notes SET archived_by_sync = ?3
             WHERE note_id = ?1 AND peer_vault_id = ?2",
            params![note_id, peer_vault_id, archived],
        )?;
        Ok(changed > 0)
    }

    /// Remove every row: git sync was disconnected.
    pub fn gitsync_clear(&self) -> Result<()> {
        self.conn().execute("DELETE FROM gitsync_notes", [])?;
        Ok(())
    }
}
