//! Host-owned run provenance, separate from script-controlled JSON.

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
    stop_generation: Option<u64>,
}

impl PartialEq for RunContext {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RunContext {}

thread_local! {
    static CURRENT: RefCell<Option<RunContext>> = const { RefCell::new(None) };
}

impl RunContext {
    /// Mark outside-text exposure. All existing clones observe this change.
    pub fn restrict(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).restricted = true;
    }

    pub fn is_restricted(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).restricted
    }

    /// Retain the oldest admitted generation, never a later replacement.
    /// This is a provenance slot, not proof that admission was authorized.
    pub fn capture_stop_generation(&self, generation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.stop_generation = Some(
            state
                .stop_generation
                .map_or(generation, |old| old.min(generation)),
        );
    }

    pub fn stop_generation(&self) -> Option<u64> {
        self.0
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
    /// Nested work must use a clone of its parent's context, not a fresh default.
    pub fn scope<T>(&self, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<RunContext>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CURRENT.with(|slot| slot.replace(Some(self.clone()))));
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::RunContext;

    #[test]
    fn descendants_share_late_restriction_and_oldest_generation() {
        let parent = RunContext::default();
        let children: Vec<_> = (0..8).map(|_| parent.clone()).collect();
        parent.capture_stop_generation(20);
        std::thread::scope(|scope| {
            for (i, child) in children.iter().enumerate() {
                scope.spawn(move || {
                    child.capture_stop_generation(i as u64 + 10);
                    child.restrict();
                });
            }
        });
        parent.capture_stop_generation(100);
        for child in &children {
            assert!(child.is_restricted());
            assert_eq!(child.stop_generation(), Some(10));
            assert_eq!(child, &parent);
        }
        assert!(!RunContext::default().is_restricted());
    }

    #[test]
    fn nested_scope_restores_after_panic_and_keeps_shared_state() {
        let parent = RunContext::default();
        parent.capture_stop_generation(4);
        parent.scope(|| {
            let child = RunContext::current().unwrap();
            let result = std::panic::catch_unwind(|| {
                child.scope(|| {
                    child.restrict();
                    child.capture_stop_generation(9);
                    panic!("test scope cleanup");
                });
            });
            assert!(result.is_err());
            assert_eq!(RunContext::current(), Some(parent.clone()));
            assert!(parent.is_restricted());
            assert_eq!(parent.stop_generation(), Some(4));
        });
        assert!(RunContext::current().is_none());
    }
}
