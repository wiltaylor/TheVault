//! Explicit capability used by the server's thread runtime, never by proposal Apply.

use super::*;

/// A trusted thread-runtime write path. Ordinary note APIs cannot obtain this through input
/// or by choosing an agent author. Keep it out of tool, script and proposal handlers.
pub struct AgentThreadRuntime<'a>(&'a Writer);

impl Writer {
    /// Access the server-owned transcript writer for agent-thread runtime operations.
    pub fn agent_thread_runtime(&self) -> AgentThreadRuntime<'_> {
        AgentThreadRuntime(self)
    }
}

impl AgentThreadRuntime<'_> {
    pub fn create(&self, input: &Input, author: &Author) -> Result<Saved> {
        self.create_with_id(&ulid::Ulid::new().to_string(), input, author)
    }

    /// The runtime's deterministic-id creation path, also used to seed runtime test fixtures.
    pub fn create_with_id(&self, id: &str, input: &Input, author: &Author) -> Result<Saved> {
        let w = self.0;
        let _write = w.write_lock();
        let inner = w.lock();
        if !schema::is_ulid(id) {
            return Err(Error::BadRequest("invalid thread id".into()));
        }
        if w.locate(id)?.is_some() || !w.state.history(id)?.is_empty() {
            return Err(Error::IdTaken(id.to_owned()));
        }
        let doc = input.parse(&note_path("agent_thread", id))?;
        let change = w.prepare_put_as(&inner, id, "agent_thread", doc, None, author, None, None, true)?;
        w.apply(inner, vec![change], &|| Ok(()), author).map(first)
    }

    pub fn update(&self, id: &str, input: &Input, base: Option<&str>, author: &Author) -> Result<Saved> {
        let w = self.0;
        let _write = w.write_lock();
        let inner = w.lock();
        let (type_name, old) = w.existing(id)?;
        if type_name != "agent_thread" {
            return Err(Error::BadRequest("not an agent thread".into()));
        }
        let doc = input.parse(&note_path(&type_name, id))?;
        let change = w.prepare_put_as(&inner, id, &type_name, doc, Some(&old), author, None, None, true)?;
        let check = || w.check_base(id, &type_name, &old, base);
        w.apply(inner, vec![change], &check, author).map(first)
    }
}
