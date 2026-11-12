//! Durable unattended trigger work, including terminal records used by backfill.
use crate::{Result, State};
use rusqlite::{OptionalExtension, params};

pub const TRIGGER_QUEUE_CAP: i64 = 10_000;
const WEEK: i64 = 7 * 24 * 60 * 60 * 1000;
const RUNNING_BOUND: i64 = 2 * 60 * 60 * 1000;
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS trigger_jobs (
 id INTEGER PRIMARY KEY, agent TEXT NOT NULL, trigger TEXT NOT NULL,
 note TEXT, payload TEXT NOT NULL, queued_ms INTEGER NOT NULL, due_ms INTEGER NOT NULL,
 waits INTEGER NOT NULL DEFAULT 0, failures INTEGER NOT NULL DEFAULT 0,
 status TEXT NOT NULL DEFAULT 'waiting', message TEXT
);
CREATE INDEX IF NOT EXISTS trigger_jobs_due ON trigger_jobs(status, due_ms);
CREATE INDEX IF NOT EXISTS trigger_jobs_note ON trigger_jobs(agent, trigger, note);
CREATE TABLE IF NOT EXISTS trigger_queue_drops (agent TEXT PRIMARY KEY, count INTEGER NOT NULL, notified_ms INTEGER, last_drop_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS trigger_backfill_previews (id TEXT PRIMARY KEY, principal TEXT NOT NULL, agent TEXT NOT NULL, trigger TEXT NOT NULL, payload TEXT NOT NULL, expires_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS trigger_events (note TEXT PRIMARY KEY, at_ms INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS trigger_legacy_runs (run_id INTEGER PRIMARY KEY, agent TEXT NOT NULL, trigger TEXT NOT NULL, matched INTEGER NOT NULL);
";

/// Preserve pre-queue success records from the first user message of each recorded thread.
/// Unreadable or missing threads remain visible as unmatched history in backfill previews.
pub(crate) fn seed_legacy(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id,agent,trigger,thread,started_ms FROM agent_runs WHERE outcome='succeeded' AND id NOT IN (SELECT run_id FROM trigger_legacy_runs) ORDER BY id")?;
    let runs = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, agent, trigger, thread, at) in runs {
        let content = match thread {
            Some(thread) => conn.query_row("SELECT content FROM versions WHERE note_id=?1 AND note_type='agent_thread' ORDER BY seq DESC LIMIT 1", [thread], |r| r.get::<_,String>(0)).optional()?,
            None => None,
        };
        let note = content.as_deref().and_then(legacy_note);
        if let Some(note) = &note {
            conn.execute("INSERT INTO trigger_jobs(agent,trigger,note,payload,queued_ms,due_ms,status) SELECT ?1,?2,?3,'{}',?4,?4,'succeeded' WHERE NOT EXISTS(SELECT 1 FROM trigger_jobs WHERE agent=?1 AND trigger=?2 AND note=?3)", params![agent,trigger,note,at])?;
        }
        conn.execute(
            "INSERT INTO trigger_legacy_runs VALUES (?1,?2,?3,?4)",
            params![id, agent, trigger, note.is_some()],
        )?;
    }
    Ok(())
}

fn legacy_note(content: &str) -> Option<String> {
    let doc = schema::Doc::parse(content, "legacy agent thread").ok()?;
    let transcript: serde_json::Value =
        serde_json::from_str(doc.field("transcript")?.as_str()?).ok()?;
    let user = transcript["messages"]
        .as_array()?
        .iter()
        .find(|m| m["role"] == "user")?;
    let text = user["content"]
        .as_array()?
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for line in text.lines().rev() {
        for op in ["create", "update", "delete"] {
            if let Some(rest) = line.strip_prefix(&format!("Event: {op} of note ")) {
                let (id, _) = rest.split_once(" (type ")?;
                id.parse::<ulid::Ulid>().ok()?;
                return Some(id.to_owned());
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct TriggerJob {
    pub id: i64,
    pub agent: String,
    pub trigger: String,
    pub note: Option<String>,
    pub payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_f6_success_survives_later_unsuccessful_jobs_and_retry_exhaustion() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        for status in ["failed", "expired", "cancelled", "exhausted"] {
            let success = s
                .enqueue_trigger("clerk", "opened", Some(status), "{}", 0)
                .unwrap()
                .unwrap();
            s.finish_trigger_job(success, "succeeded", None).unwrap();
            let later = s
                .enqueue_trigger("clerk", "opened", Some(status), "{}", 0)
                .unwrap()
                .unwrap();
            if status == "exhausted" {
                for _ in 0..4 {
                    s.retry_trigger_job(later, s.now(), Some("failed")).unwrap();
                }
            } else {
                s.finish_trigger_job(later, status, None).unwrap();
            }
            assert!(
                s.trigger_record_exists("clerk", "opened", status).unwrap(),
                "{status} erased success"
            );
            assert!(
                s.enqueue_backfill("clerk", "opened", Some(status), "{}", 0)
                    .unwrap()
                    .is_none()
            );
        }
        let rows: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM trigger_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 8, "at most success plus latest terminal per note");
    }

    #[test]
    fn review_m8_same_agent_note_runs_serially_but_other_notes_can_start() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let now = s.now();
        let first = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(first).unwrap();
        assert!(s.begin_trigger_job(first, now).unwrap());
        let second = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(second).unwrap();
        assert!(
            !s.begin_trigger_job(second, now).unwrap(),
            "same note is already running"
        );
        let other = s
            .enqueue_trigger("clerk", "opened", Some("b"), "{}", 0)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(other).unwrap();
        assert!(s.begin_trigger_job(other, now).unwrap());
        s.finish_trigger_job(first, "succeeded", None).unwrap();
        assert!(s.due_trigger_jobs(now).unwrap().is_empty());
        assert_eq!(s.due_trigger_jobs(now + 30_000).unwrap().len(), 1);
        assert!(s.claim_trigger_job(second).unwrap());
        assert!(s.begin_trigger_job(second, now + 30_000).unwrap());
        assert!(
            !s.begin_trigger_job(second, now + 30_000).unwrap(),
            "start exactly once"
        );
    }

    #[test]
    fn stale_running_note_stops_blocking_after_two_hours() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let now = s.now();
        let first = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", now)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(first).unwrap();
        assert!(s.begin_trigger_job(first, now).unwrap());
        let second = s
            .enqueue_trigger("clerk", "changed", Some("a"), "{}", now)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(second).unwrap();
        assert!(
            !s.begin_trigger_job(second, now + 7_199_999).unwrap(),
            "fresh running row blocks"
        );
        s.claim_trigger_job(second).unwrap();
        assert!(
            s.begin_trigger_job(second, now + 7_200_000).unwrap(),
            "stale running row must not block"
        );
    }

    #[test]
    fn review_f2_edit_during_running_job_is_not_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let id = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(id).unwrap();
        s.begin_trigger_job(id, s.now()).unwrap();
        assert!(
            s.enqueue_trigger("clerk", "opened", Some("a"), "{\"title\":\"new\"}", 0)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn review_f4_terminal_rows_and_old_previews_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        for _ in 0..700 {
            let id = s
                .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
                .unwrap()
                .unwrap();
            s.finish_trigger_job(id, "succeeded", None).unwrap();
            let id = s
                .enqueue_trigger("clerk", "deleted", None, "{}", 0)
                .unwrap()
                .unwrap();
            s.finish_trigger_job(id, "succeeded", None).unwrap();
        }
        for _ in 0..50 {
            s.preview_trigger_backfill("owner", "clerk", "opened", "[]")
                .unwrap();
        }
        s.conn()
            .execute(
                "UPDATE trigger_backfill_previews SET expires_ms=?1",
                [s.now() - WEEK - 1],
            )
            .unwrap();
        s.due_trigger_jobs(s.now()).unwrap();
        let jobs: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM trigger_jobs", [], |r| r.get(0))
            .unwrap();
        let previews: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM trigger_backfill_previews", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!((jobs, previews), (1, 0));
    }

    #[test]
    fn review_f5_v20_run_seeds_its_note_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let s = State::open(&path, None).unwrap();
        let note = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let transcript = serde_json::json!({"messages":[{"role":"user", "content":[{"type":"text", "text":format!("Tidy.\nEvent: create of note {note} (type memo, title Roof)")}]}]});
        let content = format!(
            "transcript = {}\n",
            serde_json::json!(transcript.to_string())
        );
        s.record_version(
            "thread",
            "agent_thread",
            &crate::Author::Agent("clerk".into()),
            &content,
        )
        .unwrap();
        let run = s.start_agent_run("clerk", "opened").unwrap();
        s.set_agent_run_thread(run, "thread").unwrap();
        s.finish_agent_run(run, &crate::AgentRunOutcome::Succeeded, "")
            .unwrap();
        let unmatched = s.start_agent_run("clerk", "opened").unwrap();
        s.finish_agent_run(unmatched, &crate::AgentRunOutcome::Succeeded, "")
            .unwrap();
        s.conn().execute_batch("DROP TABLE trigger_jobs; DROP TABLE trigger_queue_drops; DROP TABLE trigger_backfill_previews; DROP TABLE trigger_events;").unwrap();
        s.conn().pragma_update(None, "user_version", 20).unwrap();
        drop(s);
        let s = State::open(&path, None).unwrap();
        assert!(s.trigger_record_exists("clerk", "opened", note).unwrap());
        assert_eq!(
            s.unmatched_legacy_trigger_runs("clerk", "opened").unwrap(),
            1
        );
    }

    #[test]
    fn waiting_payload_updates_and_unsuccessful_notes_can_be_backfilled() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let id = s
            .enqueue_trigger("clerk", "opened", Some("a"), "old", 0)
            .unwrap()
            .unwrap();
        assert!(
            s.enqueue_trigger("clerk", "opened", Some("a"), "new", 0)
                .unwrap()
                .is_none()
        );
        assert_eq!(s.due_trigger_jobs(s.now()).unwrap()[0].payload, "new");
        for status in ["failed", "expired", "cancelled"] {
            s.finish_trigger_job(id, status, None).unwrap();
            assert!(!s.trigger_record_exists("clerk", "opened", "a").unwrap());
        }
        assert!(
            s.enqueue_backfill("clerk", "opened", Some("a"), "new", 0)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn backfill_atomically_refuses_a_note_with_a_completed_record() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let id = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
            .unwrap()
            .unwrap();
        s.finish_trigger_job(id, "succeeded", None).unwrap();
        assert!(
            s.enqueue_backfill("clerk", "opened", Some("a"), "{}", 0)
                .unwrap()
                .is_none()
        );
        assert!(
            s.enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
                .unwrap()
                .is_some(),
            "a later ordinary edit may trigger again"
        );
    }

    #[test]
    fn cap_is_visible_and_notice_is_hourly() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        for _ in 0..TRIGGER_QUEUE_CAP {
            assert!(
                s.enqueue_trigger("clerk", "opened", None, "{}", 0)
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            s.enqueue_trigger("clerk", "opened", None, "{}", 0)
                .unwrap()
                .is_none()
        );
        let stats = s.trigger_queue_stats("clerk").unwrap();
        assert_eq!((stats.waiting, stats.dropped), (10_000, 1));
        let now = s.now();
        assert!(s.trigger_drop_notice("clerk", now).unwrap());
        assert!(!s.trigger_drop_notice("clerk", now + 1).unwrap());
        assert!(
            !s.trigger_drop_notice("clerk", now + 3_600_000).unwrap(),
            "an old drop must not notify forever"
        );
        s.conn()
            .execute(
                "UPDATE trigger_queue_drops SET last_drop_ms=?1",
                [now + 3_600_001],
            )
            .unwrap();
        assert!(s.trigger_drop_notice("clerk", now + 3_600_001).unwrap());
    }

    #[test]
    fn waiting_jobs_survive_restart_but_unknown_provider_outcomes_are_not_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let s = State::open(&path, None).unwrap();
        let a = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
            .unwrap()
            .unwrap();
        let b = s
            .enqueue_trigger("clerk", "opened", Some("b"), "{}", 0)
            .unwrap()
            .unwrap();
        s.claim_trigger_job(a).unwrap();
        s.claim_trigger_job(b).unwrap();
        s.finish_trigger_job(b, "running", None).unwrap();
        drop(s);
        let s = State::open(&path, None).unwrap();
        s.recover_trigger_jobs().unwrap();
        let due = s.due_trigger_jobs(0).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, a);
        assert_eq!(s.trigger_queue_stats("clerk").unwrap().failed, 1);
    }

    #[test]
    fn capacity_backoff_expiry_and_three_failure_retries() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        let id = s
            .enqueue_trigger("clerk", "opened", Some("a"), "{}", 0)
            .unwrap()
            .unwrap();
        s.retry_trigger_job(id, 0, None).unwrap();
        assert!(s.due_trigger_jobs(29_999).unwrap().is_empty());
        assert_eq!(s.due_trigger_jobs(30_000).unwrap().len(), 1);
        s.retry_trigger_job(id, 30_000, None).unwrap();
        assert!(s.due_trigger_jobs(89_999).unwrap().is_empty());
        assert_eq!(s.due_trigger_jobs(90_000).unwrap().len(), 1);
        for i in 0..4 {
            s.retry_trigger_job(id, 100_000, Some("provider failed"))
                .unwrap();
            assert_eq!(
                s.trigger_queue_stats("clerk").unwrap().failed,
                i64::from(i == 3)
            );
        }
        let id = s
            .enqueue_trigger("clerk", "opened", Some("b"), "{}", 0)
            .unwrap()
            .unwrap();
        s.conn()
            .execute("UPDATE trigger_jobs SET queued_ms=0 WHERE id=?1", [id])
            .unwrap();
        assert!(s.due_trigger_jobs(WEEK).unwrap().is_empty());
        assert_eq!(s.trigger_queue_stats("clerk").unwrap().expired, 1);
    }

    #[test]
    fn upgrades_from_19_20_and_22_preserve_existing_runs() {
        for version in [19, 20, 22] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.db");
            let s = State::open(&path, None).unwrap();
            let run = s.start_agent_run("clerk", "opened").unwrap();
            s.conn().execute_batch("DROP TABLE trigger_jobs; DROP TABLE trigger_queue_drops; DROP TABLE trigger_backfill_previews; DROP TABLE trigger_events;").unwrap();
            if version == 19 {
                s.conn()
                    .execute_batch("DROP TABLE agent_call_ids; DROP TABLE agent_call_refusals;")
                    .unwrap();
            }
            s.conn()
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(s);
            let s = State::open(&path, None).unwrap();
            assert_eq!(s.agent_run(run).unwrap().unwrap().agent, "clerk");
            assert!(
                s.enqueue_trigger("clerk", "opened", None, "{}", 0)
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[derive(Debug, Default)]
pub struct TriggerQueueStats {
    pub waiting: i64,
    pub failed: i64,
    pub expired: i64,
    pub dropped: i64,
}

fn prune_terminal(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute("DELETE FROM trigger_jobs WHERE status IN ('succeeded','failed','expired','cancelled') AND (note IS NULL OR (id NOT IN (SELECT max(id) FROM trigger_jobs WHERE note IS NOT NULL AND status IN ('succeeded','failed','expired','cancelled') GROUP BY agent,trigger,note) AND id NOT IN (SELECT max(id) FROM trigger_jobs WHERE note IS NOT NULL AND status='succeeded' GROUP BY agent,trigger,note)))", [])?;
    Ok(())
}

impl State {
    pub fn unmatched_legacy_trigger_runs(&self, agent: &str, trigger: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM trigger_legacy_runs WHERE agent=?1 AND trigger=?2 AND matched=0",
            params![agent, trigger],
            |r| r.get(0),
        )?)
    }
    pub fn save_trigger_event(&self, note: &str, at: i64, payload: &str) -> Result<()> {
        self.conn().execute("INSERT INTO trigger_events VALUES (?1,?2,?3) ON CONFLICT(note) DO UPDATE SET at_ms=excluded.at_ms,payload=excluded.payload", params![note,at,payload])?;
        Ok(())
    }

    pub fn trigger_events(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT payload FROM trigger_events ORDER BY at_ms,note")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn acknowledge_trigger_event(&self, note: &str, before: i64) -> Result<()> {
        self.conn().execute(
            "DELETE FROM trigger_events WHERE note=?1 AND at_ms<=?2",
            params![note, before],
        )?;
        Ok(())
    }

    pub fn prune_trigger_events(&self, before: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM trigger_events WHERE at_ms<=?1", [before])?;
        Ok(())
    }

    pub fn recent_agent_starts(&self, since: i64) -> Result<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT agent,started_ms FROM agent_runs WHERE started_ms>?1 ORDER BY started_ms",
        )?;
        Ok(stmt
            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn preview_trigger_backfill(
        &self,
        principal: &str,
        agent: &str,
        trigger: &str,
        payload: &str,
    ) -> Result<String> {
        self.conn().execute(
            "DELETE FROM trigger_backfill_previews WHERE expires_ms<?1",
            [self.now() - WEEK],
        )?;
        let id = self.new_ulid().to_string();
        self.conn().execute(
            "INSERT INTO trigger_backfill_previews VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, principal, agent, trigger, payload, self.now() + 600_000],
        )?;
        Ok(id)
    }

    pub fn confirm_trigger_backfill(
        &self,
        id: &str,
        principal: &str,
        agent: &str,
        trigger: &str,
    ) -> Result<Option<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let payload = tx.query_row("SELECT payload FROM trigger_backfill_previews WHERE id=?1 AND principal=?2 AND agent=?3 AND trigger=?4 AND expires_ms>?5", params![id,principal,agent,trigger,self.now()], |r| r.get(0)).optional()?;
        if payload.is_some() {
            tx.execute("DELETE FROM trigger_backfill_previews WHERE id=?1", [id])?;
        }
        tx.commit()?;
        Ok(payload)
    }

    /// Atomically enforce the per-agent cap and note deduplication. None means no new job.
    pub fn enqueue_trigger(
        &self,
        agent: &str,
        trigger: &str,
        note: Option<&str>,
        payload: &str,
        due: i64,
    ) -> Result<Option<i64>> {
        self.enqueue_trigger_inner(agent, trigger, note, payload, due, false)
    }

    pub fn enqueue_backfill(
        &self,
        agent: &str,
        trigger: &str,
        note: Option<&str>,
        payload: &str,
        due: i64,
    ) -> Result<Option<i64>> {
        self.enqueue_trigger_inner(agent, trigger, note, payload, due, true)
    }

    fn enqueue_trigger_inner(
        &self,
        agent: &str,
        trigger: &str,
        note: Option<&str>,
        payload: &str,
        due: i64,
        only_missing: bool,
    ) -> Result<Option<i64>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if let Some(note) = note {
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM trigger_jobs WHERE agent=?1 AND trigger=?2 AND note=?3 AND ((?4 AND status IN ('succeeded','waiting','admitted','running')) OR (NOT ?4 AND status IN ('waiting','admitted'))))", params![agent, trigger, note, only_missing], |r| r.get(0))?;
            if exists {
                if !only_missing {
                    tx.execute("UPDATE trigger_jobs SET payload=?4,due_ms=max(due_ms,?5) WHERE agent=?1 AND trigger=?2 AND note=?3 AND status='waiting'", params![agent,trigger,note,payload,due])?;
                }
                tx.commit()?;
                return Ok(None);
            }
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM trigger_jobs WHERE agent=?1 AND status IN ('waiting','admitted')",
            [agent],
            |r| r.get(0),
        )?;
        if count >= TRIGGER_QUEUE_CAP {
            tx.execute("INSERT INTO trigger_queue_drops(agent,count,last_drop_ms) VALUES (?1,1,?2) ON CONFLICT(agent) DO UPDATE SET count=count+1,last_drop_ms=?2", params![agent,self.now()])?;
            tx.commit()?;
            return Ok(None);
        }
        tx.execute("INSERT INTO trigger_jobs(agent,trigger,note,payload,queued_ms,due_ms) VALUES (?1,?2,?3,?4,?5,?6)", params![agent,trigger,note,payload,self.now(),due])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some(id))
    }

    pub fn trigger_record_exists(&self, agent: &str, trigger: &str, note: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM trigger_jobs WHERE agent=?1 AND trigger=?2 AND note=?3 AND status IN ('succeeded','waiting','admitted','running'))",
            params![agent, trigger, note],
            |r| r.get(0),
        )?)
    }

    /// Claims are serialized with admission by the scheduler. Running rows are not replayed
    /// after a restart: the provider may have completed before the process stopped.
    pub fn recover_trigger_jobs(&self) -> Result<()> {
        self.conn().execute(
            "UPDATE trigger_jobs SET status='waiting' WHERE status='admitted'",
            [],
        )?;
        self.conn().execute("UPDATE trigger_jobs SET status='failed', message='interrupted run, outcome unknown; not replayed' WHERE status='running'", [])?;
        prune_terminal(&self.conn())?;
        Ok(())
    }

    pub fn due_trigger_jobs(&self, now: i64) -> Result<Vec<TriggerJob>> {
        let conn = self.conn();
        conn.execute("UPDATE trigger_jobs SET status='expired', message='wait exceeded seven days' WHERE status='waiting' AND queued_ms<=?1", [now - WEEK])?;
        prune_terminal(&conn)?;
        conn.execute(
            "DELETE FROM trigger_backfill_previews WHERE expires_ms<?1",
            [now - WEEK],
        )?;
        let mut stmt = conn.prepare("SELECT id,agent,trigger,note,payload FROM trigger_jobs WHERE status='waiting' AND due_ms<=?1 ORDER BY id LIMIT 1000")?;
        Ok(stmt
            .query_map([now], |r| {
                Ok(TriggerJob {
                    id: r.get(0)?,
                    agent: r.get(1)?,
                    trigger: r.get(2)?,
                    note: r.get(3)?,
                    payload: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn next_trigger_due(&self) -> Result<Option<i64>> {
        Ok(self.conn().query_row(
            "SELECT min(due_ms) FROM trigger_jobs WHERE status='waiting'",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn claim_trigger_job(&self, id: i64) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE trigger_jobs SET status='admitted' WHERE id=?1 AND status='waiting'",
            [id],
        )? == 1)
    }

    /// Running rows use due_ms as their start time. A lost finish must not block forever.
    pub fn trigger_note_running(&self, agent: &str, note: Option<&str>, now: i64) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM trigger_jobs WHERE agent=?1 AND note=?2 AND status='running' AND due_ms>?3)",
            params![agent, note, now - RUNNING_BOUND],
            |r| r.get(0),
        )?)
    }

    pub fn begin_trigger_job(&self, id: i64, now: i64) -> Result<bool> {
        let conn = self.conn();
        conn.execute("UPDATE trigger_jobs SET status='expired',message='wait exceeded seven days' WHERE id=?1 AND status='admitted' AND queued_ms<=?2", params![id,now-WEEK])?;
        prune_terminal(&conn)?;
        let started = conn.execute(
            "UPDATE trigger_jobs SET status='running',due_ms=?2 WHERE id=?1 AND status='admitted' AND NOT EXISTS (SELECT 1 FROM trigger_jobs other WHERE other.status='running' AND other.agent=trigger_jobs.agent AND other.note=trigger_jobs.note AND other.due_ms>?3)",
            params![id, now, now - RUNNING_BOUND],
        )? == 1;
        if !started {
            let waits: Option<u32> = conn
                .query_row(
                    "SELECT waits FROM trigger_jobs WHERE id=?1 AND status='admitted'",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(waits) = waits {
                let delay = (30_000_i64 * (1_i64 << waits.min(5))).min(900_000);
                conn.execute("UPDATE trigger_jobs SET status='waiting', due_ms=?2, waits=waits+1 WHERE id=?1 AND status='admitted'", params![id,now+delay])?;
            }
        }
        Ok(started)
    }

    pub fn finish_trigger_job(&self, id: i64, status: &str, message: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE trigger_jobs SET status=?2, message=?3 WHERE id=?1",
            params![id, status, message],
        )?;
        prune_terminal(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Capacity waits do not consume failure attempts. Three retries follow an initial failure.
    pub fn retry_trigger_job(&self, id: i64, now: i64, failure: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (waits, failures): (u32, u32) = tx.query_row(
            "SELECT waits,failures FROM trigger_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let n = if failure.is_some() { failures } else { waits };
        let delay = (30_000_i64 * (1_i64 << n.min(5))).min(900_000);
        let status = if failure.is_some() && failures >= 3 {
            "failed"
        } else {
            "waiting"
        };
        tx.execute("UPDATE trigger_jobs SET status=?2, due_ms=?3, waits=waits+?4, failures=failures+?5, message=?6 WHERE id=?1", params![id,status,now+delay,i64::from(failure.is_none()),i64::from(failure.is_some()),failure])?;
        prune_terminal(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn trigger_queue_stats(&self, agent: &str) -> Result<TriggerQueueStats> {
        let conn = self.conn();
        let count = |status| {
            conn.query_row(
                "SELECT count(*) FROM trigger_jobs WHERE agent=?1 AND status=?2",
                params![agent, status],
                |r| r.get(0),
            )
        };
        Ok(TriggerQueueStats {
            waiting: count("waiting")? + count("admitted")?,
            failed: count("failed")?,
            expired: count("expired")?,
            dropped: conn
                .query_row(
                    "SELECT count FROM trigger_queue_drops WHERE agent=?1",
                    [agent],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0),
        })
    }

    pub fn trigger_drop_notice(&self, agent: &str, now: i64) -> Result<bool> {
        Ok(self.conn().execute("UPDATE trigger_queue_drops SET notified_ms=?2 WHERE agent=?1 AND (notified_ms IS NULL OR (last_drop_ms>notified_ms AND notified_ms<=?2-3600000))", params![agent,now])? == 1)
    }
}
