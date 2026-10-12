//! The durable queue of `after_*` handler jobs (D9).
//!
//! The write path queues a job for every handler of every note it changed, after its commit. A
//! job stays `queued` until it has run, so a server that stops with jobs in the queue runs them
//! when it starts again. A job that ran is removed; one that failed stays, with its message and
//! log, as the record of the failure.

use rusqlite::params;

use crate::{Error, Result, State};

/// Where a job is in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    /// Failed, with the error message.
    Failed(String),
}

/// One handler job. The payload is the caller's, opaque to `state.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerJob {
    pub id: i64,
    pub payload: String,
    pub queued_ms: i64,
    pub status: JobStatus,
    pub ended_ms: Option<i64>,
    pub log: String,
}

impl State {
    /// Queue one job per payload, in order, in one transaction.
    pub fn queue_jobs(&self, payloads: &[String]) -> Result<Vec<i64>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = self.now();
        let mut ids = Vec::with_capacity(payloads.len());
        for p in payloads {
            tx.execute(
                "INSERT INTO handler_jobs (payload, queued_ms) VALUES (?1, ?2)",
                params![p, now],
            )?;
            ids.push(tx.last_insert_rowid());
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Record a job that failed before it could be queued, such as one past the depth limit.
    pub fn record_failed_job(&self, payload: &str, message: &str) -> Result<i64> {
        let conn = self.conn();
        let now = self.now();
        conn.execute(
            "INSERT INTO handler_jobs (payload, queued_ms, status, ended_ms, message)
             VALUES (?1, ?2, 'failed', ?2, ?3)",
            params![payload, now, message],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Every queued job, oldest first.
    pub fn queued_jobs(&self) -> Result<Vec<HandlerJob>> {
        self.jobs_where("status = 'queued'")
    }

    /// Every failed job, oldest first.
    pub fn failed_jobs(&self) -> Result<Vec<HandlerJob>> {
        self.jobs_where("status = 'failed'")
    }

    /// End queued job `id`: remove it when it ran, or keep it as failed with `log`.
    pub fn finish_job(
        &self,
        id: i64,
        outcome: std::result::Result<(), String>,
        log: &str,
    ) -> Result<()> {
        let conn = self.conn();
        let changed = match outcome {
            Ok(()) => conn.execute(
                "DELETE FROM handler_jobs WHERE id = ?1 AND status = 'queued'",
                [id],
            )?,
            Err(message) => conn.execute(
                "UPDATE handler_jobs SET status = 'failed', ended_ms = ?2, message = ?3, log = ?4
                 WHERE id = ?1 AND status = 'queued'",
                params![id, self.now(), message, log],
            )?,
        };
        if changed == 0 {
            return Err(Error::Corrupt(format!("handler job {id} is not queued")));
        }
        Ok(())
    }

    fn jobs_where(&self, filter: &str) -> Result<Vec<HandlerJob>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, payload, queued_ms, status, ended_ms, message, log FROM handler_jobs
             WHERE {filter} ORDER BY id"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, payload, queued_ms, status, ended_ms, message, log) = row?;
            let status = match status.as_str() {
                "queued" => JobStatus::Queued,
                "failed" => JobStatus::Failed(message.unwrap_or_default()),
                other => {
                    return Err(Error::Corrupt(format!(
                        "handler job {id} has status {other:?}"
                    )));
                }
            };
            out.push(HandlerJob {
                id,
                payload,
                queued_ms,
                status,
                ended_ms,
                log,
            });
        }
        Ok(out)
    }
}
