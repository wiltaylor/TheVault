//! Emptying the note trash: notes purged for good, with their history (ai D21, D13).
//!
//! Only the signed-in user purges; the API checks the caller. A purged note is gone from
//! `state.db` (its trash entry, every version and every device's base of it) and its pin is
//! removed. The index never holds a trashed note, so it has nothing to forget. An asset that
//! only purged notes named is then unused, and goes to the asset trash as a cleanup would move
//! it there.

use std::collections::BTreeSet;

use state::TrashEntry;

use crate::asset_use::ulids_in;
use crate::{Asset, Error, Result, Writer};

/// What a purge removed.
#[derive(Debug, Clone, Default)]
pub struct Purged {
    /// The notes purged, as they sat in the trash.
    pub notes: Vec<TrashEntry>,
    /// The assets only those notes used, now in the asset trash.
    pub assets: Vec<Asset>,
}

impl Writer {
    /// Purge every note in the trash for good.
    pub fn empty_trash(&self) -> Result<Purged> {
        let _w = self.write_lock();
        let entries = self.state.list_trash()?;
        self.purge_locked(entries)
    }

    /// Purge note `id` from the trash for good. A note not in the trash is
    /// [`Error::NotInTrash`].
    pub fn purge_from_trash(&self, id: &str) -> Result<Purged> {
        let _w = self.write_lock();
        let entry = self
            .state
            .trash_entry(id)?
            .ok_or_else(|| Error::NotInTrash(id.to_owned()))?;
        self.purge_locked(vec![entry])
    }

    fn purge_locked(&self, entries: Vec<TrashEntry>) -> Result<Purged> {
        let mut named = BTreeSet::new();
        let mut notes = Vec::with_capacity(entries.len());
        for e in entries {
            if !self.state.purge_note(&e.note_id)? {
                continue;
            }
            self.state.unpin(&e.note_id)?;
            named.extend(ulids_in(&e.content).into_iter().map(str::to_owned));
            notes.push(e);
        }
        let assets = if named.is_empty() {
            Vec::new()
        } else {
            let named: Vec<String> = named.into_iter().collect();
            self.cleanup_assets_locked(Some(&named), false)?
        };
        Ok(Purged { notes, assets })
    }
}
