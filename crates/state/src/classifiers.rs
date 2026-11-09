//! Pending create snapshots, consumed before an attempt so provider failures never retry.
use rusqlite::OptionalExtension;

use crate::{Result, State};

impl State {
    pub fn queue_classifications(&self, payloads: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for payload in payloads {
            tx.execute(
                "INSERT INTO classifier_pending (payload) VALUES (?1)",
                [payload],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// At-most-once dispatch. A stop during HTTP does not replay a potentially sent push.
    pub fn take_classification(&self) -> Result<Option<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let next: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, payload FROM classifier_pending ORDER BY id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, _)) = &next {
            tx.execute("DELETE FROM classifier_pending WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(next.map(|(_, payload)| payload))
    }
}
