//! The runs of agent triggers (agents G12): which agent and trigger, the thread the run wrote,
//! when it started and ended, how it ended and its trail lines (ai D12).

use rusqlite::{OptionalExtension, params};

use crate::{Error, Result, State};

pub type AgentRunId = i64;

/// How an agent run ended, or that it has not yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRunOutcome {
    Running,
    Succeeded,
    /// Failed, with the error message.
    Failed(String),
}

impl AgentRunOutcome {
    /// The outcome as `state.db` and the API name it.
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRunOutcome::Running => "running",
            AgentRunOutcome::Succeeded => "succeeded",
            AgentRunOutcome::Failed(_) => "failed",
        }
    }

    /// The failure message, for a failed run.
    pub fn message(&self) -> Option<&str> {
        match self {
            AgentRunOutcome::Failed(m) => Some(m),
            _ => None,
        }
    }
}

/// One run of an agent trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRun {
    pub id: AgentRunId,
    pub agent: String,
    pub trigger: String,
    /// The thread note the run wrote, once it has one.
    pub thread: Option<String>,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub outcome: AgentRunOutcome,
    /// The run's trail lines (ai D12), one per line.
    pub log: String,
}

const COLUMNS: &str = "id, agent, trigger, thread, started_ms, ended_ms, outcome, message, log";

type Row = (AgentRun, String, Option<String>);

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok((
        AgentRun {
            id: r.get(0)?,
            agent: r.get(1)?,
            trigger: r.get(2)?,
            thread: r.get(3)?,
            started_ms: r.get(4)?,
            ended_ms: r.get(5)?,
            outcome: AgentRunOutcome::Running,
            log: r.get(8)?,
        },
        r.get(6)?,
        r.get(7)?,
    ))
}

fn finish_row((mut run, outcome, message): Row) -> Result<AgentRun> {
    run.outcome = match outcome.as_str() {
        "running" => AgentRunOutcome::Running,
        "succeeded" => AgentRunOutcome::Succeeded,
        "failed" => AgentRunOutcome::Failed(message.unwrap_or_default()),
        other => {
            return Err(Error::Corrupt(format!(
                "agent run {} has outcome {other:?}",
                run.id
            )));
        }
    };
    Ok(run)
}

impl State {
    /// Records that a run of `agent`'s trigger `trigger` started.
    pub fn start_agent_run(&self, agent: &str, trigger: &str) -> Result<AgentRunId> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO agent_runs (agent, trigger, started_ms, outcome)
             VALUES (?1, ?2, ?3, 'running')",
            params![agent, trigger, self.now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Records the thread run `id` writes.
    pub fn set_agent_run_thread(&self, id: AgentRunId, thread: &str) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE agent_runs SET thread = ?2 WHERE id = ?1",
            params![id, thread],
        )?;
        match changed {
            0 => Err(Error::UnknownRun(id)),
            _ => Ok(()),
        }
    }

    /// Records how run `id` ended, with its trail lines. A run finishes once.
    pub fn finish_agent_run(
        &self,
        id: AgentRunId,
        outcome: &AgentRunOutcome,
        log: &str,
    ) -> Result<()> {
        if *outcome == AgentRunOutcome::Running {
            return Err(Error::Corrupt("a run cannot finish as running".into()));
        }
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE agent_runs SET ended_ms = ?2, outcome = ?3, message = ?4, log = ?5
             WHERE id = ?1 AND outcome = 'running'",
            params![id, self.now(), outcome.as_str(), outcome.message(), log],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let exists = conn
            .query_row("SELECT 1 FROM agent_runs WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some();
        Err(if exists {
            Error::RunFinished(id)
        } else {
            Error::UnknownRun(id)
        })
    }

    /// One agent run by id.
    pub fn agent_run(&self, id: AgentRunId) -> Result<Option<AgentRun>> {
        let found = self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM agent_runs WHERE id = ?1"),
                [id],
                row,
            )
            .optional()?;
        found.map(finish_row).transpose()
    }

    /// The most recent runs of `agent`'s trigger `trigger`, newest first.
    pub fn agent_runs(&self, agent: &str, trigger: &str, limit: u32) -> Result<Vec<AgentRun>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM agent_runs WHERE agent = ?1 AND trigger = ?2
             ORDER BY id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![agent, trigger, limit], row)?;
        rows.map(|r| finish_row(r?)).collect()
    }

    /// Mark every run still recorded as running failed, as the server starts: no run outlives
    /// the process that ran it.
    pub fn end_interrupted_agent_runs(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE agent_runs SET ended_ms = ?1, outcome = 'failed',
                 message = 'the server stopped during the run'
             WHERE outcome = 'running'",
            [self.now()],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_recorded_with_its_thread_and_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let a = s.start_agent_run("triage", "nightly").unwrap();
        s.set_agent_run_thread(a, "01THREAD").unwrap();
        s.finish_agent_run(a, &AgentRunOutcome::Succeeded, "line one")
            .unwrap();
        let b = s.start_agent_run("triage", "nightly").unwrap();
        s.finish_agent_run(b, &AgentRunOutcome::Failed("no key".into()), "")
            .unwrap();
        assert!(matches!(
            s.finish_agent_run(b, &AgentRunOutcome::Succeeded, ""),
            Err(Error::RunFinished(_))
        ));
        s.start_agent_run("triage", "other").unwrap();
        let runs = s.agent_runs("triage", "nightly", 20).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].outcome, AgentRunOutcome::Failed("no key".into()));
        assert_eq!(runs[1].thread.as_deref(), Some("01THREAD"));
        assert_eq!(runs[1].log, "line one");
        assert_eq!(s.end_interrupted_agent_runs().unwrap(), 1);
        let other = s.agent_runs("triage", "other", 20).unwrap();
        assert!(matches!(other[0].outcome, AgentRunOutcome::Failed(_)));
    }
}
