//! Pending create snapshots, consumed before an attempt so provider failures never retry.
use rusqlite::OptionalExtension;

use crate::{Result, State};

impl State {
    /// Activate once, recording server-observed existing IDs rather than trusting note dates.
    pub fn activate_agent_safety(&self, at: i64, existing: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='agent_safety.activation_ms')",
            [],
            |r| r.get(0),
        )?;
        if !active {
            for id in existing {
                tx.execute(
                    "INSERT OR IGNORE INTO meta(key,value) VALUES (?1,'1')",
                    [format!("agent_safety.legacy.{id}")],
                )?;
            }
            tx.execute(
                "INSERT INTO meta(key,value) VALUES ('agent_safety.activation_ms',?1)",
                [at.to_string()],
            )?;
            tx.execute("INSERT OR IGNORE INTO meta(key,value) VALUES ('agent_safety.pre_existing_mode','held')", [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist the verdict and its single arrival event together. Subsequent releases do not replay it.
    pub fn set_agent_safety_result(&self, id: &str, value: &str, cleared: bool) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [format!("agent_safety.note.{id}"), value.to_owned()])?;
        if cleared {
            let first = tx.execute(
                "INSERT OR IGNORE INTO meta(key,value) VALUES (?1,'1')",
                [format!("agent_safety.delivered.{id}")],
            )?;
            if first == 1 {
                tx.execute(
                    "INSERT INTO meta(key,value) VALUES (?1,?2)",
                    [format!("agent_safety.clearance.{id}"), id.to_owned()],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn take_agent_safety_clearances(&self) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let ids = {
            let mut stmt = tx.prepare(
                "SELECT value FROM meta WHERE key LIKE 'agent_safety.clearance.%' ORDER BY key",
            )?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?
        };
        tx.execute(
            "DELETE FROM meta WHERE key LIKE 'agent_safety.clearance.%'",
            [],
        )?;
        tx.commit()?;
        Ok(ids)
    }

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
