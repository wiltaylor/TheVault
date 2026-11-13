//! Durable automation admission and the local commit linearization boundary.
//! Use the writer-owned State for all transitions and commits. The mutex is
//! process-local, not a cross-process vault write lock.

use std::sync::MutexGuard;

use crate::{Error, Result, State};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS automation_control (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    stopped INTEGER NOT NULL CHECK(stopped IN (0, 1)),
    generation INTEGER NOT NULL CHECK(generation >= 0)
);
INSERT OR IGNORE INTO automation_control VALUES (1, 0, 0);
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationState {
    pub stopped: bool,
    pub generation: i64,
}

/// An admission identity. Resume never makes a pre-Stop identity valid again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation(i64);

impl State {
    pub fn automation_state(&self) -> Result<AutomationState> {
        Ok(self.conn().query_row(
            "SELECT stopped, generation FROM automation_control WHERE singleton=1",
            [],
            |row| {
                Ok(AutomationState {
                    stopped: row.get(0)?,
                    generation: row.get(1)?,
                })
            },
        )?)
    }

    fn automation_lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.automation_gate
            .lock()
            .map_err(|_| Error::Corrupt("automation gate poisoned".into()))
    }

    pub fn admit_automation(&self) -> Result<Generation> {
        let _gate = self.automation_lock()?;
        let state = self.automation_state()?;
        if state.stopped {
            return Err(Error::AutomationRefused);
        }
        Ok(Generation(state.generation))
    }

    /// Increment on each transition, including repeated Stop/Resume requests.
    /// No notification or caller-visible success precedes durable persistence.
    pub fn set_automation_stopped(&self, stopped: bool) -> Result<AutomationState> {
        let _gate = self.automation_lock()?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let generation: i64 = tx.query_row(
            "SELECT generation FROM automation_control WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let generation = generation
            .checked_add(1)
            .ok_or_else(|| Error::Corrupt("automation generation exhausted".into()))?;
        tx.execute(
            "UPDATE automation_control SET stopped=?1, generation=?2 WHERE singleton=1",
            rusqlite::params![stopped, generation],
        )?;
        tx.commit()?;
        Ok(AutomationState {
            stopped,
            generation,
        })
    }

    /// Hold only across local mutation, never callbacks, handlers or network I/O.
    pub fn automation_commit(&self, generation: Generation) -> Result<MutexGuard<'_, ()>> {
        let gate = self.automation_lock()?;
        let state = self.automation_state()?;
        if state.stopped || state.generation != generation.0 {
            return Err(Error::AutomationRefused);
        }
        Ok(gate)
    }
}
