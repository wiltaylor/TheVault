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
    agent_reads: bool,
    observed_outside: bool,
    stop_generation: Option<Generation>,
    parent: Option<RunContext>,
    durable: Vec<state::RunProvenanceLease>,
}

impl Drop for State {
    fn drop(&mut self) {
        // Leases move to the root on every join. The final Arc owner is gone,
        // so no live descendant can gain more provenance. No JOIN/State lock
        // is held during this database transaction. Failure leaves every row
        // unsealed and recovery restricted/stale, never a partial clean seal.
        if let Err(error) = state::RunProvenanceLease::seal_all_with_exposure(
            &self.durable,
            self.restricted,
            self.stop_generation,
            self.agent_reads,
            self.observed_outside,
        ) {
            eprintln!("sealing run provenance: {error}");
            for lease in &self.durable {
                if let Err(error) = lease.record_seal_failure() {
                    eprintln!("recording failed run seal: {error}");
                }
            }
        }
    }
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
        a.agent_reads |= b.agent_reads;
        a.observed_outside |= b.observed_outside;
        a.restricted |= a.agent_reads && a.observed_outside;
        a.stop_generation = oldest(a.stop_generation, b.stop_generation);
        a.durable.append(&mut b.durable);
        b.parent = Some(root.clone());
    }

    /// Merge host-owned queue provenance without inheriting an unrelated caller.
    pub fn merge(&self, other: &Self) {
        self.join(other);
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

    /// This run supplies content to an agent, including through task/handler descendants.
    /// Independent import tasks and direct owner reads do not opt into this policy.
    pub fn track_agent_reads(&self) {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        let root = self.root();
        let mut state = root.0.lock().unwrap_or_else(|e| e.into_inner());
        state.agent_reads = true;
        state.restricted |= state.observed_outside;
    }

    /// Record a host-classified incoming read, even before this job first uses AI.
    /// Independent non-AI imports keep working, but later model use cannot erase it.
    pub fn observe_outside_text(&self) {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        let root = self.root();
        let mut state = root.0.lock().unwrap_or_else(|e| e.into_inner());
        state.observed_outside = true;
        state.restricted |= state.agent_reads;
    }

    pub fn tracks_agent_reads(&self) -> bool {
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        self.root()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .agent_reads
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

    /// Persist an active marker before publishing a durable work reference.
    /// All later mutations stay live until the last context owner disappears,
    /// when the complete snapshot is sealed atomically. A crash or failed seal
    /// restores restricted AND stale, including failed late-generation joins.
    pub fn persist(&self, database: &state::State) -> state::Result<String> {
        // Capture before publication, not after restart. This observes Stop
        // without refusing work, and never replaces an older admission.
        self.capture_stop_generation(database.run_provenance_generation()?);
        let (restricted, generation) = {
            let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
            let root = self.root();
            let state = root.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(lease) = state
                .durable
                .iter()
                .find(|lease| lease.belongs_to(database))
            {
                return Ok(lease.id().to_owned());
            }
            (state.restricted, state.stop_generation)
        };
        // Never hold JOIN while waiting for SQLite or the writer.
        let lease = database.lease_run_provenance(restricted, generation)?;
        let id = lease.id().to_owned();
        let _join = JOIN.lock().unwrap_or_else(|e| e.into_inner());
        self.root()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .durable
            .push(lease);
        Ok(id)
    }

    /// Recover outside unrelated ambient scopes. Unknown/unreadable storage
    /// returns an error, never a runnable context without a lease. The failed
    /// recovery is recorded in activity. Queue owners must retain the queued
    /// record on error (including audit failure) and use a live-context registry.
    pub fn recover(database: &state::State, id: &str) -> state::Result<Self> {
        Self::recover_for(database, id, id)
    }

    /// The host queue supplies its subject, never the script's claimed identity.
    pub fn recover_for(database: &state::State, id: &str, subject: &str) -> state::Result<Self> {
        let (snapshot, lease) = match database.resume_run_provenance(id) {
            Ok(recovered) => recovered,
            Err(error) => {
                database.record_unavailable_run_for(id, subject)?;
                return Err(error);
            }
        };
        // Own the lease before the fallible audit. An audit failure drops this
        // context and releases its token with the conservative recovered values.
        let context = Self(Arc::new(Mutex::new(State {
            restricted: snapshot.is_restricted(),
            agent_reads: snapshot.tracks_agent_reads(),
            observed_outside: snapshot.observed_outside_text(),
            stop_generation: snapshot.stop_generation(),
            parent: None,
            durable: vec![lease],
        })));
        if snapshot
            .stop_generation()
            .is_none_or(|generation| database.automation_commit(generation).is_err())
        {
            database.record_stale_run_for(id, subject)?;
        }
        Ok(context)
    }

    /// Capture the synchronous caller's context before crossing a worker boundary.
    pub fn current() -> Option<Self> {
        CURRENT.with(|slot| slot.borrow().clone())
    }

    /// Join explicit provenance with the synchronous caller before a worker boundary.
    pub fn inherit_current(self) -> Self {
        if let Some(parent) = Self::current() {
            parent.join(&self);
        }
        self
    }

    /// Install this context for synchronous host callbacks, restoring on panic.
    /// Never return an async future from this closure or hold this scope over await.
    /// Unrelated nested contexts join permanently, including existing clones.
    pub fn scope<T>(&self, f: impl FnOnce() -> T) -> T {
        if let Some(parent) = Self::current() {
            parent.join(self);
        }
        self.scope_recovered(f)
    }

    /// Host recovery boundary for an independently recorded queue/Outbox item.
    /// Unlike `scope`, this does not join an unrelated ambient run. Never use it
    /// for descendants or script-selected contexts, nor across an await.
    pub fn scope_recovered<T>(&self, f: impl FnOnce() -> T) -> T {
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
    fn pre_model_exposure_survives_join_and_recovery_without_restricting_imports() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let import = RunContext::default();
        import.observe_outside_text();
        assert!(!import.is_restricted());
        let id = import.persist(&database).unwrap();
        drop(import);
        let recovered = RunContext::recover(&database, &id).unwrap();
        assert!(!recovered.is_restricted());
        let model = RunContext::default();
        model.track_agent_reads();
        model.merge(&recovered);
        assert!(model.is_restricted());
        assert!(recovered.is_restricted());
        let late = RunContext::default();
        late.track_agent_reads();
        late.observe_outside_text();
        assert!(late.is_restricted());
        assert!(!RunContext::default().is_restricted());
    }

    #[test]
    fn agent_read_lineage_joins_late_and_survives_sealed_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let database = state::State::open(&path, None).unwrap();
        let parent = RunContext::default();
        let child = RunContext::default();
        let id = child.persist(&database).unwrap();
        parent.scope(|| child.scope(|| {}));
        parent.track_agent_reads();
        assert!(child.tracks_agent_reads());
        assert!(!child.is_restricted());
        drop(parent);
        drop(child);
        drop(database);
        let database = state::State::open(&path, None).unwrap();
        let recovered = RunContext::recover(&database, &id).unwrap();
        assert!(recovered.tracks_agent_reads());
        assert!(!recovered.is_restricted());
        let independent = RunContext::default();
        let id = independent.persist(&database).unwrap();
        drop(independent);
        let independent = RunContext::recover(&database, &id).unwrap();
        assert!(!independent.tracks_agent_reads());
        assert!(!independent.is_restricted());
    }

    #[test]
    fn independent_recovery_scope_restores_caller_without_joining_on_panic() {
        let caller = RunContext::default();
        let recovered = RunContext::default();
        recovered.restrict();
        caller.scope(|| {
            let result = std::panic::catch_unwind(|| {
                recovered.scope_recovered(|| {
                    assert_eq!(RunContext::current(), Some(recovered.clone()));
                    panic!("injected recovery failure");
                });
            });
            assert!(result.is_err());
            assert_eq!(RunContext::current(), Some(caller.clone()));
            assert!(!caller.is_restricted());
            assert_ne!(caller, recovered);
        });
        assert!(RunContext::current().is_none());
    }

    #[test]
    fn recovered_component_is_not_sealed_while_another_owner_is_live() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let a = RunContext::default();
        let b = RunContext::default();
        let a_id = a.persist(&database).unwrap();
        let b_id = b.persist(&database).unwrap();
        a.scope(|| b.scope(|| {}));
        drop(a);
        drop(b);
        let a = RunContext::recover(&database, &a_id).unwrap();
        let b = RunContext::recover(&database, &b_id).unwrap();
        drop(a);
        b.restrict();
        for id in [&a_id, &b_id] {
            let snapshot = database.load_run_provenance(id);
            assert!(!snapshot.is_sealed());
            assert!(snapshot.is_restricted());
            assert!(
                database
                    .automation_commit(snapshot.stop_generation().unwrap())
                    .is_err()
            );
        }
        drop(b);
        for id in [&a_id, &b_id] {
            assert!(database.load_run_provenance(id).is_sealed());
            assert!(database.load_run_provenance(id).is_restricted());
        }
    }

    #[test]
    fn concurrent_persist_and_join_keep_every_reference_restricted() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let parent = RunContext::default();
        let children: Vec<_> = (0..16).map(|_| RunContext::default()).collect();
        let barrier = std::sync::Barrier::new(children.len());
        let ids = std::thread::scope(|scope| {
            let threads: Vec<_> = children
                .iter()
                .map(|child| {
                    let database = &database;
                    let parent = &parent;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        let id = child.persist(database).unwrap();
                        parent.scope(|| child.scope(|| child.restrict()));
                        id
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>()
        });
        drop(children);
        drop(parent);
        for id in ids {
            let snapshot = database.load_run_provenance(&id);
            assert!(snapshot.is_sealed());
            assert!(snapshot.is_restricted());
        }
    }

    #[test]
    fn durable_identity_seals_only_after_last_live_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let database = state::State::open(&path, None).unwrap();
        let generation = database.admit_automation().unwrap();
        let parent = RunContext::default();
        parent.capture_stop_generation(generation);
        let id = parent.persist(&database).unwrap();
        assert_eq!(parent.persist(&database).unwrap(), id);
        let child = parent.clone();
        parent.restrict(); // after publication, not a snapshot at enqueue
        drop(parent);
        let active = database.load_run_provenance(&id);
        assert!(!active.is_sealed());
        assert!(active.is_restricted());
        assert!(
            database
                .automation_commit(active.stop_generation().unwrap())
                .is_err()
        );
        drop(child);
        drop(database);
        let database = state::State::open(&path, None).unwrap();
        let sealed = database.load_run_provenance(&id);
        assert!(sealed.is_sealed());
        assert!(sealed.is_restricted());
        assert_eq!(sealed.stop_generation(), Some(generation));
    }

    #[test]
    fn durable_join_seals_every_old_reference_with_oldest_generation() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let old = database.admit_automation().unwrap();
        database.set_automation_stopped(false).unwrap();
        let new = database.admit_automation().unwrap();
        let a = RunContext::default();
        let b = RunContext::default();
        a.capture_stop_generation(new);
        b.capture_stop_generation(old);
        let a_id = a.persist(&database).unwrap();
        let b_id = b.persist(&database).unwrap();
        a.scope(|| b.scope(|| b.restrict()));
        drop(a);
        assert!(!database.load_run_provenance(&a_id).is_sealed());
        drop(b);
        for id in [&a_id, &b_id] {
            let sealed = database.load_run_provenance(id);
            assert!(sealed.is_sealed());
            assert!(sealed.is_restricted());
            assert_eq!(sealed.stop_generation(), Some(old));
            assert!(database.automation_commit(old).is_err());
        }
    }

    #[test]
    fn sealed_unrestricted_recovery_rearms_before_new_exposure() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let context = RunContext::default();
        let generation = database.admit_automation().unwrap();
        let id = context.persist(&database).unwrap();
        drop(context);
        assert!(!database.load_run_provenance(&id).is_restricted());
        let recovered = RunContext::recover(&database, &id).unwrap();
        assert!(!recovered.is_restricted());
        assert_eq!(recovered.stop_generation(), Some(generation));
        assert!(!database.load_run_provenance(&id).is_sealed());
        recovered.restrict();
        drop(recovered);
        assert!(database.load_run_provenance(&id).is_restricted());
        assert!(database.load_run_provenance(&id).is_sealed());
    }

    #[test]
    fn persisted_admission_cannot_be_revived_by_stop_and_resume() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let before = RunContext::default();
        let before_id = before.persist(&database).unwrap();
        drop(before);
        database.set_automation_stopped(true).unwrap();
        let during = RunContext::default();
        let during_id = during.persist(&database).unwrap(); // provenance, not enforcement
        drop(during);
        database.set_automation_stopped(false).unwrap();
        for id in [before_id, during_id] {
            let recovered = RunContext::recover(&database, &id).unwrap();
            recovered.capture_stop_generation(database.admit_automation().unwrap());
            assert!(
                database
                    .automation_commit(recovered.stop_generation().unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn unknown_and_interrupted_recovery_never_acquire_fresh_admission() {
        let dir = tempfile::tempdir().unwrap();
        let database = state::State::open(&dir.path().join("state.db"), None).unwrap();
        let generation = database.admit_automation().unwrap();
        let active = database
            .create_run_provenance(false, Some(generation))
            .unwrap();
        assert!(RunContext::recover(&database, "missing").is_err());
        let activity = database
            .activity(&state::ActivityFilter::default())
            .unwrap();
        assert_eq!(activity.items[0].kind, "run.recovery_unavailable");
        assert_eq!(activity.items[0].subject.as_deref(), Some("missing"));
        {
            let id = active.id();
            let recovered = RunContext::recover(&database, id).unwrap();
            assert!(recovered.is_restricted());
            assert!(
                database
                    .automation_commit(recovered.stop_generation().unwrap())
                    .is_err()
            );
            recovered.capture_stop_generation(generation);
            assert!(
                database
                    .automation_commit(recovered.stop_generation().unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn mixed_storage_authorities_leave_unsealed_records_instead_of_partial_seal() {
        let dir = tempfile::tempdir().unwrap();
        let a_db = state::State::open(&dir.path().join("a.db"), None).unwrap();
        let b_db = state::State::open(&dir.path().join("b.db"), None).unwrap();
        let a = RunContext::default();
        let b = RunContext::default();
        let a_id = a.persist(&a_db).unwrap();
        let b_id = b.persist(&b_db).unwrap();
        a.scope(|| b.scope(|| {}));
        drop(a);
        drop(b);
        assert!(!a_db.load_run_provenance(&a_id).is_sealed());
        assert!(!b_db.load_run_provenance(&b_id).is_sealed());
        for (database, id) in [(&a_db, &a_id), (&b_db, &b_id)] {
            let activity = database
                .activity(&state::ActivityFilter::default())
                .unwrap();
            assert_eq!(activity.items.len(), 1);
            assert_eq!(activity.items[0].kind, "run.seal_failed");
            assert_eq!(activity.items[0].subject.as_ref(), Some(id));
        }
    }

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
