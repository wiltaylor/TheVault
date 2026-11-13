//! Host-owned durable run identity. Never put this record in script payload JSON.
//!
//! Queue rows retain the ID originally allocated to them. Joining two records
//! links roots, so old references continue to see every later restriction and
//! the oldest admission. The database transaction serializes joins and updates.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

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
CREATE TABLE IF NOT EXISTS run_provenance_leases (
    token TEXT PRIMARY KEY,
    run_id TEXT NOT NULL
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

    /// Fail-closed provenance when the caller cannot safely read or re-arm it.
    /// This issues only a permanently stale identity, never a valid admission.
    pub fn unavailable(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            restricted: true,
            generation: Some(Generation::stale()),
            sealed: false,
        }
    }
}

/// An unsealed durable identity owned by one live RunContext component.
/// Not cloneable: transfer it on live joins, then seal all owned identities
/// together when the last live context disappears. Dropping without sealing is
/// safe and deliberately leaves recovery restricted/stale.
/// Lease tokens are durable: a leaked or crashed owner cannot be mistaken for
/// a completed owner, even through another connection or after restart. Such
/// a component remains unsealed permanently rather than guessing it is safe.
pub struct RunProvenanceLease {
    id: String,
    token: String,
    conn: Arc<Mutex<Connection>>,
    clock: Arc<dyn crate::Clock>,
}

impl std::fmt::Debug for RunProvenanceLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunProvenanceLease")
            .field("id", &self.id)
            .finish()
    }
}

impl RunProvenanceLease {
    /// Record a failed final seal in the owner's activity log. If storage itself
    /// is unavailable, the durable lease remains unsealed for recovery to report.
    pub fn record_seal_failure(&self) -> Result<()> {
        record_failure(
            &mut self.conn.lock().unwrap_or_else(|e| e.into_inner()),
            self.clock.now_ms(),
            &self.id,
            "run.seal_failed",
            "Run provenance could not be sealed; recovery remains restricted and stale",
            None,
        )
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn belongs_to(&self, state: &State) -> bool {
        Arc::ptr_eq(&self.conn, &state.conn)
    }

    /// Atomically store the final live snapshot, join all durable references,
    /// and seal. Call only after every live owner is gone. Failure rolls back
    /// the whole seal, so no reference can restore a partial final snapshot.
    pub fn seal_all(
        leases: &[Self],
        mut restricted: bool,
        mut generation: Option<Generation>,
    ) -> Result<()> {
        let Some(first) = leases.first() else {
            return Ok(());
        };
        if leases
            .iter()
            .any(|lease| !Arc::ptr_eq(&lease.conn, &first.conn))
        {
            return Err(Error::Corrupt(
                "run provenance uses multiple State authorities".into(),
            ));
        }
        let mut conn = first.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        let mut roots = BTreeSet::new();
        for lease in leases {
            if tx.execute(
                "DELETE FROM run_provenance_leases WHERE token=?1 AND run_id=?2",
                params![lease.token, lease.id],
            )? != 1
            {
                return Err(Error::Corrupt(
                    "run provenance lease already released".into(),
                ));
            }
            let (root, snapshot) = resolve(&tx, &lease.id)?;
            restricted |= snapshot.restricted;
            generation = oldest(generation, snapshot.generation);
            roots.insert(root);
        }
        let root = roots.first().expect("at least one lease");
        for other in &roots {
            tx.execute(
                "UPDATE run_provenance SET restricted=?2,stop_generation=?3,parent=?4 WHERE id=?1",
                params![
                    other,
                    restricted,
                    generation.map(Generation::stored),
                    (other != root).then_some(root)
                ],
            )?;
        }
        if !has_live_leases(&tx, root)? {
            seal_component(&tx, root)?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn record_failure(
    conn: &mut Connection,
    now: i64,
    run_id: &str,
    kind: &str,
    summary: &str,
    once: Option<&str>,
) -> Result<()> {
    let tx = conn.transaction()?;
    if let Some(subject) = once {
        let key = format!("run-audit:{kind}:{subject}");
        if tx.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES (?1,'1')",
            [key],
        )? == 0
        {
            return Ok(());
        }
    }
    let id = crate::activity::next_id(&tx, now)?;
    tx.execute(
        "INSERT INTO activity(id,at_ms,kind,category,actor_kind,actor,summary,subject_kind,subject)
         VALUES (?1,?2,?3,'automation','server','server',?4,'run',?5)",
        params![id, now, kind, summary, run_id],
    )?;
    tx.commit()?;
    Ok(())
}

fn seal_component(conn: &Connection, root: &str) -> Result<()> {
    if has_live_leases(conn, root)? {
        return Err(Error::Corrupt(
            "run provenance still has live leases".into(),
        ));
    }
    conn.execute(
        "WITH RECURSIVE members(id) AS (
            SELECT ?1 UNION SELECT p.id FROM run_provenance p JOIN members m ON p.parent=m.id
         ) UPDATE run_provenance SET sealed=1 WHERE id IN (SELECT id FROM members)",
        [root],
    )?;
    Ok(())
}

fn has_live_leases(conn: &Connection, root: &str) -> Result<bool> {
    Ok(conn.query_row(
        "WITH RECURSIVE members(id) AS (
            SELECT ?1 UNION SELECT p.id FROM run_provenance p JOIN members m ON p.parent=m.id
         ) SELECT EXISTS(SELECT 1 FROM run_provenance_leases
                         WHERE run_id IN (SELECT id FROM members))",
        [root],
        |row| row.get(0),
    )?)
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
    /// Canonical host identity for live registries. Resolve validates every link.
    pub fn run_provenance_root(&self, id: &str) -> Result<String> {
        resolve(&self.conn(), id).map(|(root, _)| root)
    }

    pub fn queued_run_provenance(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT p.run_id FROM trigger_provenance p JOIN trigger_jobs j ON j.id=p.job_id WHERE j.status IN ('waiting','admitted','running') UNION SELECT p.run_id FROM handler_provenance p JOIN handler_jobs j ON j.id=p.job_id WHERE j.status='queued' UNION SELECT run_id FROM outbox_provenance")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn record_stale_run(&self, id: &str) -> Result<()> {
        self.record_stale_run_for(id, id)
    }

    pub fn record_stale_run_for(&self, id: &str, subject: &str) -> Result<()> {
        record_failure(
            &mut self.conn(),
            self.now(),
            id,
            "run.recovery_stale",
            &format!(
                "{subject}: queued run recovered with a stale admission; its original provenance was retained. Outbox delivery stays held until an owner edit after Resume"
            ),
            Some(subject),
        )
    }

    /// Fallible audit before a queue consumes unavailable work. On failure the
    /// caller must leave the queue record pending, never execute it unaudited.
    pub fn record_unavailable_run(&self, id: &str) -> Result<()> {
        self.record_unavailable_run_for(id, id)
    }

    pub fn record_unavailable_run_for(&self, id: &str, subject: &str) -> Result<()> {
        record_failure(
            &mut self.conn(),
            self.now(),
            id,
            "run.recovery_unavailable",
            &format!(
                "{subject}: queued work is held because its provenance is unavailable; owner action is required"
            ),
            Some(subject),
        )
    }

    /// Capture provenance without enforcing Stop. Work created while stopped
    /// gets a permanently stale identity, never an admission revived by Resume.
    pub fn run_provenance_generation(&self) -> Result<Generation> {
        let state = self.automation_state()?;
        if state.stopped {
            Ok(Generation::stale())
        } else {
            Generation::restore_stored(state.generation)
                .ok_or_else(|| Error::Corrupt("invalid automation generation".into()))
        }
    }

    /// Start an unsealed lifecycle before a queue publishes this ID.
    pub fn lease_run_provenance(
        &self,
        restricted: bool,
        generation: Option<Generation>,
    ) -> Result<RunProvenanceLease> {
        let snapshot = self.create_run_provenance(restricted, generation)?;
        let token = ulid::Ulid::new().to_string();
        self.conn().execute(
            "INSERT INTO run_provenance_leases(token,run_id) VALUES (?1,?2)",
            params![token, snapshot.id],
        )?;
        Ok(RunProvenanceLease {
            id: snapshot.id,
            token,
            conn: self.conn.clone(),
            clock: self.clock.clone(),
        })
    }

    /// Recover and re-arm an existing identity in one transaction, before any
    /// new live owner can gain provenance. Owners must reuse their live context
    /// registry instead of repeatedly resuming the same ID within a process.
    pub fn resume_run_provenance(&self, id: &str) -> Result<(RunProvenance, RunProvenanceLease)> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (root, mut snapshot) = resolve(&tx, id)?;
        if !snapshot.sealed || snapshot.generation.is_none() {
            snapshot.restricted = true;
            snapshot.generation = Some(Generation::stale());
        }
        tx.execute("UPDATE run_provenance SET sealed=0 WHERE id=?1", [root])?;
        let token = ulid::Ulid::new().to_string();
        tx.execute(
            "INSERT INTO run_provenance_leases(token,run_id) VALUES (?1,?2)",
            params![token, id],
        )?;
        tx.commit()?;
        Ok((
            snapshot,
            RunProvenanceLease {
                id: id.to_owned(),
                token,
                conn: self.conn.clone(),
                clock: self.clock.clone(),
            },
        ))
    }

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
    /// happened after its last successful write. It is also stale: a later join
    /// may have lowered its generation without reaching storage. Sealed records
    /// restore exactly as sealed. Recovery callers must durably record stale
    /// work for the owner, never silently discard an interrupted automation.
    pub fn load_run_provenance(&self, id: &str) -> RunProvenance {
        resolve(&self.conn(), id)
            .map(|(_, mut snapshot)| {
                if !snapshot.sealed {
                    snapshot.restricted = true;
                    snapshot.generation = Some(Generation::stale());
                }
                snapshot
            })
            .unwrap_or_else(|_| RunProvenance::unavailable(id))
    }

    /// Seal only after no live context can gain restriction or join another run.
    /// This is a host lifecycle operation, not a script or queue-payload option.
    /// A failed seal leaves recovery conservative. Sealing an entire component
    /// requires that all its live owners have finished, not merely one job.
    pub fn seal_run_provenance(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (root, _) = resolve(&tx, id)?;
        seal_component(&tx, &root)?;
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
