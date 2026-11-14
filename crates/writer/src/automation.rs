//! Explicit synchronous run context. Step 2 supplies it at worker entry points.
use std::cell::RefCell;
use std::sync::MutexGuard;

use state::automation::Generation;

use crate::{Result, Writer};

#[cfg(test)]
mod completion_tests {
    use super::*;
    use script_core::Vault as _;
    use serde_json::json;

    #[test]
    fn completion_cannot_authorize_other_writes_or_refresh_admission() {
        let dir = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
        let writer = Writer::open(vault, state).unwrap();
        writer
            .upgrade_shipped_defaults(crate::Upkeep::Home)
            .unwrap();
        let item = writer.create("outbox", &crate::Input::Json(json!({"fields": {
            "title": "Existing attempt", "kind": "sms", "status": "sending", "phone": "+61400000000"
        }})), &state::Author::Session("owner".into())).unwrap();
        let active = script_core::RunContext::default();
        let run_id = active.persist(writer.state()).unwrap();
        let recovered = script_core::RunContext::recover(writer.state(), &run_id).unwrap();
        let generation = recovered.stop_generation().unwrap();
        let author = state::Author::Handler("outbox_send".into());
        recovered.scope_recovered(|| {
            writer.with_sms_completion(&item.id, || {
                writer.state().set_automation_stopped(true).unwrap();
                assert!(
                    writer
                        .patch(&item.id, json!({"status":"failed"}), "", &author)
                        .is_err()
                );
                writer.state().set_automation_stopped(false).unwrap();
                assert!(
                    writer
                        .patch(&item.id, json!({"title":"New action"}), "", &author)
                        .is_err()
                );
                assert!(writer.delete(&item.id, None, &author).is_err());
                writer
                    .patch(
                        &item.id,
                        json!({"status":"failed", "error":"Interrupted"}),
                        "",
                        &author,
                    )
                    .unwrap();
            });
            assert!(
                writer
                    .patch(
                        &item.id,
                        json!({"error":"ordinary stale write"}),
                        "",
                        &author
                    )
                    .is_err()
            );
        });
        assert!(writer.state().automation_commit(generation).is_err());
        assert_eq!(recovered.stop_generation(), Some(generation));
        let current = writer.get(&item.id).unwrap();
        assert_eq!(current["note"]["fields"]["title"], "Existing attempt");
        assert_eq!(current["note"]["fields"]["error"], "Interrupted");
    }
}

thread_local! {
    static GENERATIONS: RefCell<Vec<Generation>> = const { RefCell::new(Vec::new()) };
    static SMS_COMPLETION: RefCell<Option<String>> = const { RefCell::new(None) };
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
    pub(crate) fn with_sms_completion<T>(&self, item: &str, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                SMS_COMPLETION.with(|s| *s.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(SMS_COMPLETION.with(|s| s.replace(Some(item.to_owned()))));
        work()
    }

    /// The exception is checked against every final file, after before-handlers
    /// and expansion. It never changes the run's admission or reaches callbacks.
    pub(crate) fn completion_commit(
        &self,
        files: &[store::Change],
    ) -> Result<Option<MutexGuard<'_, ()>>> {
        let recovery = script_core::current_job(|job| {
            (job.source == include_str!("../defaults/outbox_send.ws")
                && matches!(job.function.as_str(), "reconcile_send" | "reconcile_delete"))
            .then(|| job.outbox_recovery.clone())
            .flatten()
        })
        .flatten();
        let sms = SMS_COMPLETION.with(|s| s.borrow().clone());
        let item = recovery
            .as_ref()
            .map(|r| r.item.as_str())
            .or(sms.as_deref());
        let Some(item) = item else {
            return self.automation_commit();
        };
        let path = crate::note_path("outbox", item);
        let old = crate::Input::Wcl(self.vault.read_text(&path)?).parse(&path)?;
        let old_text = |key: &str| old.field(key).and_then(schema::Value::as_str).unwrap_or("");
        if !matches!(old_text("status"), "sending" | "failed") {
            return Err(state::Error::AutomationRefused.into());
        }
        for file in files {
            let allowed = match file {
                store::Change::Write {
                    path: changed,
                    contents,
                } if changed == &path => {
                    let next = crate::Input::Wcl(String::from_utf8_lossy(contents).into_owned())
                        .parse(&path)?;
                    let status = next
                        .field("status")
                        .and_then(schema::Value::as_str)
                        .unwrap_or("");
                    let allowed = [
                        "sent_ref",
                        "sent_at",
                        "result",
                        "error",
                        "interrupted_at",
                        "status",
                        "updated",
                        "event",
                    ];
                    old.blocks == next.blocks
                        && matches!(status, "sending" | "failed")
                        && old.fields.iter().chain(&next.fields).all(|(key, _)| {
                            allowed.contains(&key.as_str()) || old.field(key) == next.field(key)
                        })
                        && (old.field("event") == next.field("event")
                            || next.field("event").is_none())
                }
                store::Change::Delete { path: changed } if changed == &path => {
                    old_text("status") == "sending" && !old_text("sent_at").is_empty()
                }
                store::Change::Delete { path: changed } => recovery
                    .as_ref()
                    .and_then(|r| r.delete_target.as_ref())
                    .is_some_and(|target| {
                        changed == &crate::note_path("event", target)
                            && old_text("sent_ref") == target
                            && !old_text("sent_at").is_empty()
                    }),
                _ => false,
            };
            if !allowed {
                return Err(state::Error::AutomationRefused.into());
            }
        }
        self.state
            .automation_completion()
            .map(Some)
            .map_err(Into::into)
    }

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
        self.recover_run_context_for(id, id)
    }

    pub fn recover_run_context_for(
        &self,
        id: &str,
        subject: &str,
    ) -> state::Result<script_core::RunContext> {
        let mut contexts = self
            .durable_contexts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = match self.state.run_provenance_root(id) {
            Ok(root) => root,
            Err(_) => return script_core::RunContext::recover_for(&self.state, id, subject),
        };
        if let Some(context) = contexts.get(&root) {
            if context
                .stop_generation()
                .is_none_or(|g| self.state.automation_commit(g).is_err())
            {
                self.state.record_stale_run_for(id, subject)?;
            }
            return Ok(context.clone());
        }
        let context = script_core::RunContext::recover_for(&self.state, id, subject)?;
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
        // Pruned owners seal outside this registry lock (replacement during
        // publication can seal under it). State is a leaf in the lock order.
        // Read errors keep all owners rather than guessing which are alive.
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
