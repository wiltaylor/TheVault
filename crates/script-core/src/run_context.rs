//! Host-owned run provenance, separate from script-controlled JSON.

use state::automation::Generation;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

/// Shared monotonic state for one run and all its descendants and retries.
///
/// Clone this object at every worker boundary. Creating a default context starts
/// an independent run. This type carries provenance only and refuses no action.
#[derive(Clone, Debug, Default)]
pub struct RunContext(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    restricted: bool,
    stop_generation: Option<Generation>,
    parent: Option<RunContext>,
}

// Serializes root lookup, joins and mutations. No user code, writer lock,
// database operation or await is allowed while holding this lock.
static JOIN: Mutex<()> = Mutex::new(());

impl PartialEq for RunContext {
    fn eq(&self, other: &Self) -> bool {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        Arc::ptr_eq(&self.root().0, &other.root().0)
    }
}

impl Eq for RunContext {}

thread_local! {
    static CURRENT: RefCell<Option<RunContext>> = const { RefCell::new(None) };
}

impl RunContext {
    // Caller holds JOIN. Root-to-root links cannot cycle.
    fn root(&self) -> Self {
        let mut root = self.clone();
        loop {
            let parent = root
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .parent
                .clone();
            match parent {
                Some(parent) => root = parent,
                None => return root,
            }
        }
    }

    fn join(&self, other: &Self) {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        let root = self.root();
        let other = other.root();
        if Arc::ptr_eq(&root.0, &other.0) {
            return;
        }
        let mut a = root.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = other.0.lock().unwrap_or_else(|e| e.into_inner());
        a.restricted |= b.restricted;
        a.stop_generation = oldest(a.stop_generation, b.stop_generation);
        b.parent = Some(root.clone());
    }

    /// Mark outside-text exposure. All existing clones observe this change.
    pub fn restrict(&self) {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        self.root()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .restricted = true;
    }

    pub fn is_restricted(&self) -> bool {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        self.root()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .restricted
    }

    /// Retain the oldest admitted generation, never a later replacement.
    /// Only State can issue a credential. All credentials use one State authority.
    pub fn capture_stop_generation(&self, generation: Generation) {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        let root = self.root();
        let mut state = root.0.lock().unwrap_or_else(|e| e.into_inner());
        state.stop_generation = oldest(state.stop_generation, Some(generation));
    }

    pub fn stop_generation(&self) -> Option<Generation> {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        self.root()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop_generation
    }

    /// Capture the synchronous caller's context before crossing a worker boundary.
    pub fn current() -> Option<Self> {
        CURRENT.with(|slot| slot.borrow().clone())
    }

    /// Install this context for synchronous host callbacks, restoring on panic.
    /// Never return an async future from this closure or hold this scope over await.
    /// Unrelated nested contexts join permanently, including existing clones.
    pub fn scope<T>(&self, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<RunContext>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
            }
        }
        if let Some(parent) = Self::current() {
            parent.join(self);
        }
        let _restore = Restore(CURRENT.with(|slot| slot.replace(Some(self.clone()))));
        f()
    }
}

fn oldest(a: Option<Generation>, b: Option<Generation>) -> Option<Generation> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.oldest(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::RunContext;

    #[test]
    fn descendants_share_late_restriction_and_oldest_generation() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let old = state.admit_automation().unwrap();
        state.set_automation_stopped(false).unwrap();
        let new = state.admit_automation().unwrap();
        let parent = RunContext::default();
        let children: Vec<_> = (0..8).map(|_| RunContext::default()).collect();
        parent.capture_stop_generation(old);
        std::thread::scope(|scope| {
            for child in &children {
                let parent = &parent;
                scope.spawn(move || {
                    child.capture_stop_generation(new);
                    parent.scope(|| {
                        child.scope(|| {
                            assert_eq!(RunContext::current().unwrap().stop_generation(), Some(old));
                        })
                    });
                });
            }
        });
        children[0].restrict();
        parent.capture_stop_generation(new);
        for child in &children {
            assert!(child.is_restricted());
            assert_eq!(child.stop_generation(), Some(old));
            assert_eq!(child, &parent);
        }
        assert!(!RunContext::default().is_restricted());
    }

    #[test]
    fn nested_scope_restores_after_panic_and_keeps_shared_state() {
        let parent = RunContext::default();
        parent.restrict();
        parent.scope(|| {
            let child = RunContext::default();
            let result = std::panic::catch_unwind(|| {
                child.scope(|| {
                    assert!(RunContext::current().unwrap().is_restricted());
                    panic!("test scope cleanup");
                });
            });
            assert!(result.is_err());
            assert_eq!(RunContext::current(), Some(parent.clone()));
            assert!(parent.is_restricted());
            assert!(child.is_restricted());
        });
        assert!(RunContext::current().is_none());
    }
}
