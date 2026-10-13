//! Notes pinned to the sidebar. A pin names a note by id; a pin whose note is gone stays until it
//! is removed, so restoring the note from the trash brings its pin back.

use rusqlite::params;

use crate::{Result, State};

impl State {
    /// The pinned note ids, in sidebar order.
    pub fn pins(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT note_id FROM pins ORDER BY position, pinned_ms")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// Pin `note_id` at the end of the list. Pinning a pinned note changes nothing. Returns
    /// whether the note was newly pinned.
    pub fn pin(&self, note_id: &str) -> Result<bool> {
        let now = self.now();
        let added = self.conn().execute(
            "INSERT OR IGNORE INTO pins (note_id, position, pinned_ms)
             VALUES (?1, (SELECT COALESCE(MAX(position), -1) + 1 FROM pins), ?2)",
            params![note_id, now],
        )?;
        Ok(added == 1)
    }

    /// Unpin `note_id`. Returns whether it was pinned.
    pub fn unpin(&self, note_id: &str) -> Result<bool> {
        let removed = self
            .conn()
            .execute("DELETE FROM pins WHERE note_id = ?1", [note_id])?;
        Ok(removed == 1)
    }
}

#[cfg(test)]
mod tests {
    use crate::State;

    fn open() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(&dir.path().join("state.db"), None).unwrap();
        (dir, state)
    }

    #[test]
    fn pins_keep_the_order_they_were_added_in() {
        let (_dir, s) = open();
        assert!(s.pin("b").unwrap());
        assert!(s.pin("a").unwrap());
        assert!(s.pin("c").unwrap());
        assert_eq!(s.pins().unwrap(), ["b", "a", "c"]);
    }

    #[test]
    fn pinning_twice_keeps_one_pin_in_its_first_place() {
        let (_dir, s) = open();
        s.pin("a").unwrap();
        s.pin("b").unwrap();
        assert!(!s.pin("a").unwrap());
        assert_eq!(s.pins().unwrap(), ["a", "b"]);
    }

    #[test]
    fn unpinning_removes_only_that_pin() {
        let (_dir, s) = open();
        s.pin("a").unwrap();
        s.pin("b").unwrap();
        assert!(s.unpin("a").unwrap());
        assert!(!s.unpin("a").unwrap());
        assert_eq!(s.pins().unwrap(), ["b"]);
        s.pin("a").unwrap();
        assert_eq!(s.pins().unwrap(), ["b", "a"]);
    }

    #[test]
    fn pins_survive_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        State::open(&path, None).unwrap().pin("a").unwrap();
        assert_eq!(State::open(&path, None).unwrap().pins().unwrap(), ["a"]);
    }
}
