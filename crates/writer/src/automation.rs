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
        GENERATIONS.with(|g| { g.borrow_mut().pop(); });
    }
}

impl Writer {
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
        let generation = GENERATIONS.with(|g| g.borrow().first().copied());
        generation.map(|g| self.state.automation_commit(g).map_err(Into::into)).transpose()
    }
}
