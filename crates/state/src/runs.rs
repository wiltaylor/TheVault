//! Scheduled task run records (D10).

use rusqlite::{OptionalExtension, params};

use crate::{Error, Result, State};

pub type RunId = i64;

/// How a task run ended, or that it has not yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Running,
    Succeeded,
    /// Failed, with the error message.
    Failed(String),
}

/// One run of a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRun {
    pub id: RunId,
    pub task: String,
    /// 1 for the first try, counting up through retries.
    pub attempt: u32,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub outcome: RunOutcome,
    /// Log output, one line per [`State::append_run_log`] call.
    pub log: String,
}

const RUN_COLUMNS: &str = "id, task, attempt, started_ms, ended_ms, outcome, message, log";

fn run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(TaskRun, String, Option<String>)> {
    Ok((
        TaskRun {
            id: row.get(0)?,
            task: row.get(1)?,
            attempt: row.get(2)?,
            started_ms: row.get(3)?,
            ended_ms: row.get(4)?,
            outcome: RunOutcome::Running,
            log: row.get(7)?,
        },
        row.get(5)?,
        row.get(6)?,
    ))
}

fn finish_run_row(
    (mut run, outcome, message): (TaskRun, String, Option<String>),
) -> Result<TaskRun> {
    run.outcome = match outcome.as_str() {
        "running" => RunOutcome::Running,
        "succeeded" => RunOutcome::Succeeded,
        "failed" => RunOutcome::Failed(message.unwrap_or_default()),
        other => {
            return Err(Error::Corrupt(format!(
                "run {} has outcome {other:?}",
                run.id
            )));
        }
    };
    Ok(run)
}

impl State {
    /// Records that a task run started.
    pub fn start_run(&self, task: &str, attempt: u32) -> Result<RunId> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO task_runs (task, attempt, started_ms, outcome) VALUES (?1, ?2, ?3, 'running')",
            params![task, attempt, self.now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Appends one line to a run's log.
    pub fn append_run_log(&self, id: RunId, line: &str) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE task_runs SET log = log || ?2 || char(10) WHERE id = ?1",
            params![id, line],
        )?;
        match changed {
            0 => Err(Error::UnknownRun(id)),
            _ => Ok(()),
        }
    }

    /// Records how a run ended. A run finishes once.
    pub fn finish_run(&self, id: RunId, outcome: &RunOutcome) -> Result<()> {
        let (name, message) = match outcome {
            RunOutcome::Running => {
                return Err(Error::Corrupt("a run cannot finish as running".into()));
            }
            RunOutcome::Succeeded => ("succeeded", None),
            RunOutcome::Failed(m) => ("failed", Some(m.as_str())),
        };
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE task_runs SET ended_ms = ?2, outcome = ?3, message = ?4
             WHERE id = ?1 AND outcome = 'running'",
            params![id, self.now(), name, message],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let exists = conn
            .query_row("SELECT 1 FROM task_runs WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some();
        Err(if exists {
            Error::RunFinished(id)
        } else {
            Error::UnknownRun(id)
        })
    }

    /// One run by id.
    pub fn run(&self, id: RunId) -> Result<Option<TaskRun>> {
        let row = self
            .conn()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM task_runs WHERE id = ?1"),
                [id],
                run_row,
            )
            .optional()?;
        row.map(finish_run_row).transpose()
    }

    /// A task's most recent runs, newest first.
    pub fn runs(&self, task: &str, limit: u32) -> Result<Vec<TaskRun>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM task_runs WHERE task = ?1 ORDER BY id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![task, limit], run_row)?;
        rows.map(|r| finish_run_row(r?)).collect()
    }

    /// A task's most recent run.
    pub fn latest_run(&self, task: &str) -> Result<Option<TaskRun>> {
        Ok(self.runs(task, 1)?.into_iter().next())
    }
}
