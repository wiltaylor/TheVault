//! Pending create snapshots, consumed before an attempt so provider failures never retry.
use rusqlite::OptionalExtension;

use crate::{Result, State};

impl State {
    /// Activate once, recording server-observed existing IDs rather than trusting note dates.
    pub fn activate_agent_safety(&self, at: i64, existing: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS agent_safety_index (id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, held INTEGER NOT NULL);")?;
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
                // These notes already had their arrival before activation.
                tx.execute(
                    "INSERT OR IGNORE INTO meta(key,value) VALUES (?1,'1')",
                    [format!("agent_safety.delivered.{id}")],
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
        let hash = serde_json::from_str::<serde_json::Value>(value)
            .ok()
            .and_then(|v| v["fingerprint"].as_str().map(str::to_owned))
            .unwrap_or_default();
        tx.execute("INSERT INTO agent_safety_index(id,fingerprint,held) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET fingerprint=excluded.fingerprint,held=excluded.held",
            rusqlite::params![id, hash, !cleared])?;
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

    pub fn agent_safety_clearances(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT value FROM meta WHERE key LIKE 'agent_safety.clearance.%' ORDER BY key",
        )?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn acknowledge_agent_safety_clearance(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM meta WHERE key=?1",
            [format!("agent_safety.clearance.{id}")],
        )?;
        Ok(())
    }

    pub fn index_agent_safety(&self, id: &str, hash: &str, held: bool) -> Result<()> {
        self.conn().execute("INSERT INTO agent_safety_index(id,fingerprint,held) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET fingerprint=excluded.fingerprint,held=excluded.held",
            rusqlite::params![id, hash, held])?;
        Ok(())
    }

    pub fn held_agent_note_ids(&self) -> Result<std::collections::HashSet<String>> {
        if self.meta("agent_safety.activation_ms")?.is_none() {
            return Ok(Default::default());
        }
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id FROM agent_safety_index WHERE held=1 AND NOT (
            EXISTS(SELECT 1 FROM meta WHERE key='agent_safety.pre_existing_mode' AND value='legacy_allowed')
            AND EXISTS(SELECT 1 FROM meta WHERE key='agent_safety.legacy.' || id))")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
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

    pub fn classification_queued(&self, id: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM classifier_pending WHERE json_extract(payload,'$.id')=?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    pub fn forget_agent_safety_index(&self, id: &str) -> Result<()> {
        if self.meta("agent_safety.activation_ms")?.is_some() {
            self.conn()
                .execute("DELETE FROM agent_safety_index WHERE id=?1", [id])?;
        }
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
