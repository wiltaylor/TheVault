//! Host-owned durable run identity. Never put this record in script payload JSON.
//!
//! Queue rows retain the ID originally allocated to them. Joining two records
//! links roots, so old references continue to see every later restriction and
//! the oldest admission. The database transaction serializes joins and updates.

use std::collections::BTreeSet;

use rusqlite::{Connection, params};

use crate::{Error, Result, State, automation::Generation};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run_provenance (
    id TEXT PRIMARY KEY,
    format INTEGER NOT NULL,
    restricted INTEGER NOT NULL,
    sealed INTEGER NOT NULL DEFAULT 0,
    stop_generation INTEGER,
    parent TEXT
);
";

/// A snapshot read by State, not an admission minted by a caller.
/// Re-read by ID at recovery. A snapshot does not track later changes.
#[derive(Clone, Debug)]
pub struct RunProvenance {
    id: String,
    restricted: bool,
    generation: Option<Generation>,
    sealed: bool,
}

impl RunProvenance {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_restricted(&self) -> bool {
        self.restricted
    }

    pub fn stop_generation(&self) -> Option<Generation> {
        self.generation
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    fn unknown(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            restricted: true,
            generation: Some(Generation::stale()),
            sealed: false,
        }
    }
}

fn oldest(a: Option<Generation>, b: Option<Generation>) -> Option<Generation> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.oldest(b)),
        (a, b) => a.or(b),
    }
}

// Validate every visited row, including non-root rows. Corruption of an old
// reference must not silently turn it into a fresh unrestricted admission.
fn resolve(conn: &Connection, id: &str) -> Result<(String, RunProvenance)> {
    let current: i64 = conn.query_row(
        "SELECT generation FROM automation_control WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if current < 0 {
        return Err(Error::Corrupt("invalid automation generation".into()));
    }
    let mut seen = BTreeSet::new();
    let mut cursor = id.to_owned();
    let mut snapshot = RunProvenance {
        id: id.to_owned(),
        restricted: false,
        generation: None,
        sealed: true,
    };
    loop {
        if !seen.insert(cursor.clone()) {
            return Err(Error::Corrupt("run provenance cycle".into()));
        }
        let (format, restricted, generation, parent, sealed) = conn.query_row(
            "SELECT format, restricted, stop_generation, parent, sealed FROM run_provenance WHERE id=?1",
            [&cursor],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )?;
        if format != 1
            || !matches!(restricted, 0 | 1)
            || !matches!(sealed, 0 | 1)
            || generation.is_some_and(|value| value > current)
        {
            return Err(Error::Corrupt("invalid run provenance format".into()));
        }
        let generation = generation
            .map(|value| {
                Generation::restore_stored(value)
                    .ok_or_else(|| Error::Corrupt("invalid run provenance generation".into()))
            })
            .transpose()?;
        snapshot.restricted |= restricted == 1;
        snapshot.sealed &= sealed == 1;
        snapshot.generation = oldest(snapshot.generation, generation);
        match parent {
            Some(parent) => cursor = parent,
            None => return Ok((cursor, snapshot)),
        }
    }
}

impl State {
    /// Allocate host provenance before publishing a durable work reference.
    /// The generation must be an existing State-issued credential.
    pub fn create_run_provenance(
        &self,
        restricted: bool,
        generation: Option<Generation>,
    ) -> Result<RunProvenance> {
        let id = ulid::Ulid::new().to_string();
        self.conn().execute(
            "INSERT INTO run_provenance(id,format,restricted,stop_generation) VALUES (?1,1,?2,?3)",
            params![id, restricted, generation.map(Generation::stored)],
        )?;
        Ok(RunProvenance {
            id,
            restricted,
            generation,
            sealed: false,
        })
    }

    /// Recover from host-owned storage, failing closed on any read/format error.
    /// Unknown provenance is restricted AND stale, even after Stop/Resume.
    /// An interrupted unsealed run is restricted because exposure may have
    /// happened after its last successful write. Sealed unrestricted runs stay
    /// unrestricted. A readable record retains its recorded admission identity.
    pub fn load_run_provenance(&self, id: &str) -> RunProvenance {
        resolve(&self.conn(), id)
            .map(|(_, mut snapshot)| {
                snapshot.restricted |= !snapshot.sealed;
                snapshot
            })
            .unwrap_or_else(|_| RunProvenance::unknown(id))
    }

    /// Seal only after no live context can gain restriction or join another run.
    /// This is a host lifecycle operation, not a script or queue-payload option.
    /// A failed seal leaves recovery conservative. Sealing an entire component
    /// requires that all its live owners have finished, not merely one job.
    pub fn seal_run_provenance(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (root, _) = resolve(&tx, id)?;
        tx.execute(
            "WITH RECURSIVE members(id) AS (
                SELECT ?1 UNION SELECT p.id FROM run_provenance p JOIN members m ON p.parent=m.id
             ) UPDATE run_provenance SET sealed=1 WHERE id IN (SELECT id FROM members)",
            [root],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Add restriction/admission information, never replace earlier information.
    /// Errors must prevent callers from claiming the update was persisted.
    pub fn update_run_provenance(
        &self,
        id: &str,
        restricted: bool,
        generation: Option<Generation>,
    ) -> Result<RunProvenance> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (root, mut snapshot) = resolve(&tx, id)?;
        snapshot.restricted |= restricted;
        snapshot.generation = oldest(snapshot.generation, generation);
        snapshot.sealed = false;
        tx.execute(
            "UPDATE run_provenance SET restricted=?2, stop_generation=?3, sealed=0 WHERE id=?1",
            params![
                root,
                snapshot.restricted,
                snapshot.generation.map(Generation::stored)
            ],
        )?;
        tx.commit()?;
        Ok(snapshot)
    }

    /// Permanently join durable identities. Existing references remain valid.
    /// Missing or corrupt inputs are errors, never repaired into fresh runs.
    pub fn join_run_provenance(&self, a: &str, b: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (a_root, a) = resolve(&tx, a)?;
        let (b_root, b) = resolve(&tx, b)?;
        if a_root != b_root {
            tx.execute(
                "UPDATE run_provenance SET restricted=?2, stop_generation=?3, sealed=0 WHERE id=?1",
                params![
                    a_root,
                    a.restricted || b.restricted,
                    oldest(a.generation, b.generation).map(Generation::stored)
                ],
            )?;
            tx.execute(
                "UPDATE run_provenance SET parent=?2 WHERE id=?1",
                params![b_root, a_root],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
