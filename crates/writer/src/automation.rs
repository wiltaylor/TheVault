//! Explicit synchronous run context. Step 2 supplies it at worker entry points.
use std::cell::RefCell;
use std::sync::MutexGuard;

use state::automation::Generation;

use crate::{Result, Writer};

thread_local! {
    static GENERATIONS: RefCell<Vec<Generation>> = const { RefCell::new(Vec::new()) };
}

struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        GENERATIONS.with(|g| {
            g.borrow_mut().pop();
        });
    }
}

impl Writer {
    /// Serialize durable publication and live registry insertion with recovery.
    /// The closure is host storage work only, never script/provider callbacks.
    pub fn publish_run_context<T>(
        &self,
        context: &script_core::RunContext,
        publish: impl FnOnce(&state::State, &str) -> state::Result<T>,
    ) -> state::Result<T> {
        let mut contexts = self
            .durable_contexts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let id = context.persist(&self.state)?;
        let root = self.state.run_provenance_root(&id)?;
        if let Some(previous) = contexts.get(&root) {
            context.merge(previous);
        }
        let result = publish(&self.state, &id)?;
        contexts.insert(root, context.clone());
        Ok(result)
    }

    /// Shared by agent jobs, handlers and Outbox records. Aliases of one sealed
    /// component must not independently re-arm it and falsely become stale.
    pub fn recover_run_context(&self, id: &str) -> state::Result<script_core::RunContext> {
        let mut contexts = self
            .durable_contexts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = match self.state.run_provenance_root(id) {
            Ok(root) => root,
            Err(_) => return script_core::RunContext::recover(&self.state, id),
        };
        if let Some(context) = contexts.get(&root) {
            return Ok(context.clone());
        }
        let context = script_core::RunContext::recover(&self.state, id)?;
        contexts.insert(root, context.clone());
        Ok(context)
    }

    pub fn prune_run_contexts(&self) {
        let removed = (|| -> state::Result<Vec<script_core::RunContext>> {
            let mut contexts = self
                .durable_contexts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let ids = self.state.queued_run_provenance()?;
            let roots: Vec<_> = ids
                .iter()
                .filter(|id| !id.is_empty())
                .map(|id| self.state.run_provenance_root(id))
                .collect::<state::Result<_>>()?;
            let remove: Vec<_> = contexts
                .keys()
                .filter(|id| !roots.contains(id))
                .cloned()
                .collect();
            Ok(remove
                .into_iter()
                .filter_map(|id| contexts.remove(&id))
                .collect())
        })();
        // No registry lock is held during final-owner sealing. Read errors keep
        // all owners rather than guessing which pending references are alive.
        drop(removed);
    }

    /// Run synchronous writer operations with an explicitly admitted generation.
    /// Do not return a future from this closure: context does not follow async
    /// tasks or threads. Each worker must enter its own scope. Nested scopes
    /// retain the oldest identity so they cannot revive a cancelled run.
    pub fn with_automation<T>(&self, generation: Generation, work: impl FnOnce() -> T) -> T {
        GENERATIONS.with(|g| g.borrow_mut().push(generation));
        let _scope = Scope;
        work()
    }

    pub(crate) fn automation_commit(&self) -> Result<Option<MutexGuard<'_, ()>>> {
        let explicit = GENERATIONS.with(|g| g.borrow().iter().copied().reduce(Generation::oldest));
        let inherited = script_core::RunContext::current().and_then(|c| c.stop_generation());
        let generation = match (explicit, inherited) {
            (Some(a), Some(b)) => Some(a.oldest(b)),
            (a, b) => a.or(b),
        };
        generation
            .map(|g| {
                self.with_automation(g, || self.state.automation_commit(g).map_err(Into::into))
            })
            .transpose()
    }
}
