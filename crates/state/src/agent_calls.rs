//! The calls of agents committed in notes (agents G19): each call id, made by the client that
//! wrote the call, is answered once. A call is recorded with the thread its run writes, or with
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
}
