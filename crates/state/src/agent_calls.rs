//! The calls of agents committed in notes (agents G19): server-issued, note-and-agent-bound
//! capabilities are consumed once. A call is recorded with the thread its run writes, or with
//! none when it was not run (an agent that may not be called from the note, is disabled or is
//! missing), and is never answered again.

use rusqlite::{OptionalExtension, params};

use crate::{Result, State};

/// One call of an agent from a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCall {
    pub call: String,
    pub agent: String,
    pub note: String,
    /// The thread the call's run writes; `None` for a call that was not run.
    pub thread: Option<String>,
    pub created_ms: i64,
}

impl State {
    /// Issue a short-lived capability, independent of answered calls.
    pub fn issue_agent_call(
        &self,
        call: &str,
        note: &str,
        agent: &str,
        principal: &str,
    ) -> Result<()> {
        let scope = (!principal.starts_with("session:")).then_some(crate::Scope::Write);
        self.issue_agent_call_scoped(call, note, agent, principal, scope)
    }

    /// Persist the issuing principal's ceiling. Sessions have no additional ceiling.
    pub fn issue_agent_call_scoped(
        &self,
        call: &str,
        note: &str,
        agent: &str,
        principal: &str,
        scope: Option<crate::Scope>,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO agent_call_ids (call,note,agent,principal,expires_ms,scope) VALUES (?1,?2,?3,?4,?5,?6)",
            params![call, note, agent, principal, self.now() + 86_400_000, scope.map(crate::Scope::as_str)],
        )?;
        Ok(())
    }

    /// The stored ceiling, failing closed for an unscoped non-session capability.
    pub fn agent_call_scope(&self, call: &str) -> Result<Option<crate::Scope>> {
        let (principal, scope): (String, Option<String>) = self.conn().query_row(
            "SELECT principal,scope FROM agent_call_ids WHERE call=?1",
            [call],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        match scope {
            Some(scope) => Ok(Some(scope.parse()?)),
            None if principal.starts_with("session:") => Ok(None),
            None => Ok(Some(crate::Scope::Write)),
        }
    }

    /// The persisted issuance boundary and principal for a note's reserved call.
    pub fn agent_call_issuance(
        &self,
        call: &str,
        note: &str,
        agent: &str,
    ) -> Result<Option<(i64, String)>> {
        Ok(self.conn().query_row(
            "SELECT expires_ms - 86400000, principal FROM agent_call_ids WHERE call=?1 AND note=?2 AND agent=?3",
            params![call, note, agent], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?)
    }

    /// Atomically consume an unused capability bound to this note and agent.
    pub fn consume_agent_call(
        &self,
        call: &str,
        note: &str,
        agent: &str,
        author: &crate::Author,
    ) -> Result<bool> {
        if !matches!(author, crate::Author::Session(_) | crate::Author::Token(_)) {
            return Ok(false);
        }
        let (kind, name) = author.parts();
        let principal = format!("{kind}:{name}");
        Ok(self.conn().execute(
            "UPDATE agent_call_ids SET used=1 WHERE call=?1 AND note=?2 AND agent=?3 AND principal=?4 AND used=0 AND expires_ms>?5",
            params![call, note, agent, principal, self.now()],
        )? == 1)
    }

    /// Whether a reserved id belongs elsewhere. Observing a copy must not poison it.
    pub fn agent_call_bound_elsewhere(&self, call: &str, note: &str, agent: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_call_ids WHERE call=?1 AND (note<>?2 OR agent<>?3))",
            params![call, note, agent],
            |r| r.get(0),
        )?)
    }

    /// Release an admission that opened no run. Answered ids cannot be released.
    pub fn release_agent_call(&self, call: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE agent_call_ids SET used=0 WHERE call=?1 AND NOT EXISTS(SELECT 1 FROM agent_calls WHERE call=?1)",
            [call],
        )?;
        Ok(())
    }

    /// Keep an observed copy's refusal separate from the legitimate capability's answer.
    pub fn refuse_agent_call_copy(&self, call: &str, note: &str, agent: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO agent_call_refusals (call,note,agent,created_ms) VALUES (?1,?2,?3,?4)",
            params![call, note, agent, self.now()],
        )?;
        Ok(())
    }

    pub fn refused_agent_call(&self, call: &str, note: &str) -> Result<Option<AgentCall>> {
        Ok(self
            .conn()
            .query_row(
            "SELECT call,agent,note,created_ms FROM agent_call_refusals WHERE call=?1 AND note=?2",
            params![call, note],
                |r| {
                    Ok(AgentCall {
                        call: r.get(0)?,
                        agent: r.get(1)?,
                        note: r.get(2)?,
                        thread: None,
                        created_ms: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Called by the existing activity upkeep. Answers remain as replay tombstones.
    pub fn prune_agent_calls(&self) -> Result<usize> {
        let conn = self.conn();
        let now = self.now();
        let old = now - 30 * 86_400_000;
        let ids = conn.execute("DELETE FROM agent_call_ids WHERE (used=0 AND expires_ms<=?1) OR (used=1 AND expires_ms<=?2)", params![now, old])?;
        let refusals = conn.execute(
            "DELETE FROM agent_call_refusals WHERE created_ms<=?1",
            [old],
        )?;
        Ok(ids + refusals)
    }

    /// Records call `call` of `agent` from note `note`, answered by `thread`. Answers false, and
    /// changes nothing, when the call is recorded already.
    pub fn record_agent_call(
        &self,
        call: &str,
        agent: &str,
        note: &str,
        thread: Option<&str>,
    ) -> Result<bool> {
        let changed = self.conn().execute(
            "INSERT OR IGNORE INTO agent_calls (call, agent, note, thread, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![call, agent, note, thread, self.now()],
        )?;
        Ok(changed == 1)
    }

    /// Forgets call `call`, so the next save of its note asks again. Answers false when no such
    /// call is recorded. Used when a call's run is refused after its thread was opened (TV-86 F2).
    pub fn forget_agent_call(&self, call: &str) -> Result<bool> {
        let changed = self
            .conn()
            .execute("DELETE FROM agent_calls WHERE call = ?1", [call])?;
        if changed == 1 {
            self.conn()
                .execute("UPDATE agent_call_ids SET used=0 WHERE call=?1", [call])?;
        }
        Ok(changed == 1)
    }

    /// The call recorded as `call`, if it is.
    pub fn agent_call(&self, call: &str) -> Result<Option<AgentCall>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT call, agent, note, thread, created_ms FROM agent_calls WHERE call = ?1",
                [call],
                |r| {
                    Ok(AgentCall {
                        call: r.get(0)?,
                        agent: r.get(1)?,
                        note: r.get(2)?,
                        thread: r.get(3)?,
                        created_ms: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_call_scopes_migrate_without_widening_devices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let s = State::open(&path, None).unwrap();
        s.issue_agent_call("DEVICE", "NOTE", "clerk", "token:device").unwrap();
        s.issue_agent_call("SESSION", "NOTE", "clerk", "session:owner").unwrap();
        s.conn().execute_batch("ALTER TABLE agent_call_ids DROP COLUMN scope; PRAGMA user_version=25;").unwrap();
        drop(s);
        let s = State::open(&path, None).unwrap();
        assert_eq!(s.agent_call_scope("DEVICE").unwrap(), Some(crate::Scope::Write));
        assert_eq!(s.agent_call_scope("SESSION").unwrap(), None);
    }

    #[test]
    fn issued_scope_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        s.issue_agent_call_scoped("CALL", "NOTE", "clerk", "token:device", Some(crate::Scope::Read)).unwrap();
        assert_eq!(s.agent_call_scope("CALL").unwrap(), Some(crate::Scope::Read));
    }

    #[test]
    fn issued_calls_are_bound_and_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        s.issue_agent_call("CALL", "NOTE", "clerk", "session:owner")
            .unwrap();
        for author in [
            crate::Author::Token("writer".into()),
            crate::Author::Token("admin".into()),
            crate::Author::Token("agent-clerk".into()),
            crate::Author::Token("device-other".into()),
            crate::Author::Handler("stamp".into()),
            crate::Author::Task("stamp".into()),
            crate::Author::Agent("clerk".into()),
        ] {
            assert!(
                !s.consume_agent_call("CALL", "NOTE", "clerk", &author)
                    .unwrap(),
                "{author:?}"
            );
        }
        assert!(
            !s.consume_agent_call(
                "CALL",
                "OTHER",
                "clerk",
                &crate::Author::Session("owner".into())
            )
            .unwrap()
        );
        assert!(
            !s.consume_agent_call(
                "CALL",
                "NOTE",
                "owl",
                &crate::Author::Session("owner".into())
            )
            .unwrap()
        );
        assert!(
            s.consume_agent_call(
                "CALL",
                "NOTE",
                "clerk",
                &crate::Author::Session("owner".into())
            )
            .unwrap()
        );
        assert!(
            !s.consume_agent_call(
                "CALL",
                "NOTE",
                "clerk",
                &crate::Author::Session("owner".into())
            )
            .unwrap()
        );
        assert!(
            !s.consume_agent_call(
                "FORGED",
                "NOTE",
                "clerk",
                &crate::Author::Session("owner".into())
            )
            .unwrap()
        );
    }

    #[test]
    fn a_device_issued_call_requires_that_same_device() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        s.issue_agent_call("CALL", "NOTE", "clerk", "token:device-one")
            .unwrap();
        for author in [
            crate::Author::Session("owner".into()),
            crate::Author::Token("device-two".into()),
        ] {
            assert!(
                !s.consume_agent_call("CALL", "NOTE", "clerk", &author)
                    .unwrap()
            );
        }
        assert!(
            s.consume_agent_call(
                "CALL",
                "NOTE",
                "clerk",
                &crate::Author::Token("device-one".into())
            )
            .unwrap()
        );
    }

    #[test]
    fn a_call_is_recorded_once() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        assert_eq!(s.agent_call("01CALL").unwrap(), None);
        assert!(
            s.record_agent_call("01CALL", "clerk", "01NOTE", Some("01THREAD"))
                .unwrap()
        );
        assert!(
            !s.record_agent_call("01CALL", "owl", "01OTHER", None)
                .unwrap()
        );
        let c = s.agent_call("01CALL").unwrap().unwrap();
        assert_eq!(c.agent, "clerk");
        assert_eq!(c.note, "01NOTE");
        assert_eq!(c.thread.as_deref(), Some("01THREAD"));
        assert!(
            s.record_agent_call("01NOPE", "gone", "01NOTE", None)
                .unwrap()
        );
        assert_eq!(s.agent_call("01NOPE").unwrap().unwrap().thread, None);
    }

    #[test]
    fn a_forgotten_call_is_recorded_again() {
        let dir = tempfile::tempdir().unwrap();
        let s = State::open(&dir.path().join("state.db"), None).unwrap();
        assert!(!s.forget_agent_call("01CALL").unwrap());
        s.record_agent_call("01CALL", "clerk", "01NOTE", Some("01THREAD"))
            .unwrap();
        assert!(s.forget_agent_call("01CALL").unwrap());
        assert_eq!(s.agent_call("01CALL").unwrap(), None);
        assert!(
            s.record_agent_call("01CALL", "clerk", "01NOTE", Some("01THREAD2"))
                .unwrap()
        );
    }
}
