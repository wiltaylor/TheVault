//! Agent delivery decisions are server-owned, never trusted from editable note fields.
use schema::Doc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Archived, Result, Writer, doc_to_json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Safety {
    pub status: String,
    pub reason: String,
    pub fingerprint: String,
}

impl Safety {
    pub fn allowed(&self) -> bool {
        matches!(
            self.status.as_str(),
            "allowed" | "released" | "legacy" | "not_required"
        )
    }
}

pub fn incoming(kind: &str, doc: &Doc) -> bool {
    let note = doc_to_json(doc);
    let fields = &note["fields"];
    match kind {
        "email" => !fields["labels"].as_array().is_some_and(|labels| {
            labels
                .iter()
                .any(|l| matches!(l.as_str(), Some("system/SENT" | "system/DRAFT")))
        }),
        "message" => fields["direction"] != "out",
        "call" => fields["direction"] != "out",
        "event" => fields["my_response"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        // Calendar sync retains the organiser address on the series, but not my_response.
        // Unknown ownership is conservative: only an exact own-calendar address exempts it.
        "event_series" => {
            if fields["organiser"].as_str().is_some_and(|s| !s.is_empty()) {
                fields["organiser"] != fields["calendar"]
            } else {
                fields.get("google_ref").is_some_and(|v| !v.is_null())
            }
        }
        _ => false,
    }
}

/// The same content projection is sent to Jev and hashed. Workflow and sync bookkeeping
/// are not evidence. Unknown/custom fields remain evidence rather than silently bypassing it.
pub fn classified_content(doc: &Doc) -> serde_json::Value {
    let mut note = doc_to_json(doc);
    if let Some(blocks) = note["blocks"].as_array_mut() {
        blocks.retain(|block| {
            !matches!(
                block["labels"][0].as_str(),
                Some("snapshot" | "inline_images")
            )
        });
    }
    if let Some(fields) = note["fields"].as_object_mut() {
        fields.retain(|key, _| {
            !key.ends_with("_ref")
                && !matches!(
                    key.as_str(),
                    "created"
                        | "updated"
                        | "labels"
                        | "processed"
                        | "images_pending"
                        | "inline_images"
                        | "converter"
                        | "inbox_catch_up"
                        | "etag"
                        | "snapshot"
                        | "my_response"
                        | "archived"
                )
        });
        if fields
            .get("scam_probability")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|p| (0.0..=1.0).contains(&p))
        {
            fields.remove("scam_probability");
        }
        for key in ["communication_category", "agent_safety_reason"] {
            if fields
                .get(key)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| {
                    matches!(
                        s,
                        "allow"
                            | "prompt_injection"
                            | "sensitive_credentials"
                            | "scam"
                            | "spam"
                            | "marketing"
                            | "personal"
                            | "transactional"
                            | "other"
                    )
                })
            {
                fields.remove(key);
            }
        }
    }
    note
}

pub fn fingerprint(kind: &str, doc: &Doc) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{kind}:{}", classified_content(doc)))
    )
}

impl Writer {
    pub fn set_agent_safety_wake(&self, wake: std::sync::Arc<dyn Fn() + Send + Sync>) {
        *self.safety_wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    }
    /// Reusable by explicit single/bulk user actions, never by an agent or sync token.
    pub fn release_agent_safety(
        &self,
        id: &str,
        hash: &str,
        actor: &state::Author,
    ) -> Result<bool> {
        if !matches!(actor, state::Author::Session(_)) {
            return Err(crate::Error::BadRequest(
                "Only a user session may release a note".into(),
            ));
        }
        let released = self.finish_agent_safety(id, hash, "released", "user_released")?;
        if released {
            self.state().record_activity(
                state::NewActivity::new(
                    "agent_safety.release",
                    "notes",
                    actor,
                    "Released note to agents",
                )
                .subject("note", id),
            );
        }
        Ok(released)
    }

    /// User inventory for review/backfill. Dates are the vault note creation dates, inclusive.
    pub fn held_agent_notes(
        &self,
        kind: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        reason: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        let parse = |date: &str| {
            chrono::DateTime::parse_from_rfc3339(date)
                .map_err(|_| crate::Error::BadRequest("Invalid creation date filter".into()))
        };
        let since = since.map(parse).transpose()?;
        let until = until.map(parse).transpose()?;
        let held = self.state().held_agent_note_ids()?;
        let cached = self.state().indexed_agent_safety()?;
        let notes = self.read_index(|index, _| {
            let mut stmt = index.connection().prepare("SELECT id,type,created FROM notes WHERE (?1 IS NULL OR type=?1)")?;
            stmt.query_map([kind], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))))?
                .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()
        }).map_err(index::Error::from)?;
        let mut out = Vec::new();
        for (id, hash, stored) in cached {
            if !held.contains(&id) { continue; }
            let Some((type_name, created)) = notes.get(&id) else { continue; };
            let date = created
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            if since.is_some_and(|s| date.is_none_or(|d| d < s))
                || until.is_some_and(|s| date.is_none_or(|d| d > s))
            {
                continue;
            }
            let mut safety = stored.and_then(|v| serde_json::from_str::<Safety>(&v).ok()).unwrap_or(Safety {
                status: "held".into(), reason: "unclassified".into(), fingerprint: hash.clone(),
            });
            if safety.fingerprint != hash {
                safety = Safety { status: "held".into(), reason: "content_changed".into(), fingerprint: hash };
            }
            if !safety.allowed() && reason.is_none_or(|r| r == safety.reason) {
                out.push(serde_json::json!({"id":id,"type":type_name,"created":created,"agent_safety":safety}));
            }
        }
        Ok(out)
    }

    pub fn activate_agent_safety(&self) -> Result<()> {
        if self.state().meta("agent_safety.index_ready")?.as_deref() == Some("1") {
            for (id, _, stored) in self.state().indexed_agent_safety()? {
                if let Some(mut safety) = stored.and_then(|v| serde_json::from_str::<Safety>(&v).ok())
                    && safety.status == "pending" && !self.state().classification_queued(&id)?
                {
                    safety.status = "held".into();
                    safety.reason = "unclassified".into();
                    self.state().set_agent_safety_result(&id, &serde_json::to_string(&safety).expect("safety"), false)?;
                }
            }
            return Ok(());
        }
        let existing = self
            .list_with(None, Archived::Include)?
            .into_iter()
            .map(|n| n.id)
            .collect::<Vec<_>>();
        self.state()
            .activate_agent_safety(self.now_ms(), &existing)?;
        // Pin the incoming scope of legacy notes too: editing direction must not bypass a hold.
        for id in existing {
            if self
                .state()
                .meta(&format!("agent_safety.legacy.{id}"))?
                .is_some()
                && self
                    .state()
                    .meta(&format!("agent_safety.note.{id}"))?
                    .is_none()
            {
                let note = self.read(&id)?;
                if incoming(&note.type_name, &note.doc) {
                    let safety = Safety {
                        status: "held".into(),
                        reason: "predates_activation".into(),
                        fingerprint: fingerprint(&note.type_name, &note.doc),
                    };
                    self.state().set_agent_safety_result(
                        &id,
                        &serde_json::to_string(&safety).expect("safety"),
                        false,
                    )?;
                }
            }
            let note = self.read(&id)?;
            let mut safety = self.agent_safety_doc(&id, &note.type_name, &note.doc)?;
            if safety.status == "pending" && !self.state().classification_queued(&id)? {
                safety.status = "held".into();
                safety.reason = "unclassified".into();
                self.state().set_agent_safety_result(
                    &id,
                    &serde_json::to_string(&safety).expect("safety"),
                    false,
                )?;
            }
            self.refresh_agent_safety_index(&id, &note.type_name, &note.doc)?;
        }
        self.state().set_meta("agent_safety.index_ready", "1")?;
        Ok(())
    }

    pub fn agent_safety_doc(&self, id: &str, kind: &str, doc: &Doc) -> Result<Safety> {
        let hash = fingerprint(kind, doc);
        let result = |status: &str, reason: &str| Safety {
            status: status.into(),
            reason: reason.into(),
            fingerprint: hash.clone(),
        };
        if self.state().meta("agent_safety.activation_ms")?.is_none() {
            return Ok(result("not_required", "inactive"));
        }
        // Once gated, changing direction/type cannot turn the gate off.
        let stored = self.state().meta(&format!("agent_safety.note.{id}"))?;
        if stored.is_none() && !incoming(kind, doc) {
            return Ok(result("not_required", "outgoing_or_local"));
        }
        if self
            .state()
            .meta(&format!("agent_safety.legacy.{id}"))?
            .is_some()
        {
            if self
                .state()
                .meta("agent_safety.pre_existing_mode")?
                .as_deref()
                == Some("legacy_allowed")
            {
                return Ok(result("legacy", "predates_activation"));
            }
            if stored.is_none() {
                return Ok(result("held", "predates_activation"));
            }
        }
        if let Some(stored) = stored {
            if let Ok(safety) = serde_json::from_str::<Safety>(&stored) {
                if safety.fingerprint == hash {
                    return Ok(safety);
                }
                return Ok(result("held", "content_changed"));
            }
            return Ok(result("held", "invalid_state"));
        }
        Ok(result("held", "unclassified"))
    }

    pub fn agent_safety(&self, id: &str) -> Result<Safety> {
        let note = self.read(id)?;
        self.agent_safety_doc(id, &note.type_name, &note.doc)
    }

    pub fn agent_may_read(&self, id: &str) -> bool {
        self.agent_safety(id).is_ok_and(|s| s.allowed())
    }

    /// Called with the commit lock before publishing an updated index row. No vault scan.
    pub(crate) fn refresh_agent_safety_index(&self, id: &str, kind: &str, doc: &Doc) -> Result<()> {
        if self.state().meta("agent_safety.activation_ms")?.is_some() {
            let safety = self.agent_safety_doc(id, kind, doc)?;
            if safety.reason == "unclassified" {
                self.state().set_agent_safety_result(
                    id,
                    &serde_json::to_string(&safety).expect("safety"),
                    false,
                )?;
            }
            self.state().index_agent_safety(
                id,
                &safety.fingerprint,
                !safety.allowed() || safety.status == "legacy",
            )?;
        }
        Ok(())
    }

    pub fn agent_may_read_asset(&self, id: &str) -> bool {
        self.asset_usage(id).is_ok_and(|usage| {
            usage.notes.iter().all(|n| self.agent_may_read(&n.note)) && usage.trashed.is_empty()
        })
    }

    /// Called under the existing commit lock by the after-write hook.
    pub fn register_agent_safety(&self, id: &str, kind: &str, doc: &Doc) -> Result<()> {
        if incoming(kind, doc)
            && self
                .state()
                .meta(&format!("agent_safety.note.{id}"))?
                .is_none_or(|v| {
                    serde_json::from_str::<Safety>(&v).is_ok_and(|s| s.reason == "unclassified")
                })
        {
            let safety = Safety {
                status: "pending".into(),
                reason: "awaiting_decision".into(),
                fingerprint: fingerprint(kind, doc),
            };
            self.state().set_agent_safety_result(
                id,
                &serde_json::to_string(&safety).expect("safety"),
                false,
            )?;
        }
        Ok(())
    }

    /// Compare content under the writer lock so a delayed answer cannot clear edited text.
    pub fn finish_agent_safety(
        &self,
        id: &str,
        hash: &str,
        status: &str,
        reason: &str,
    ) -> Result<bool> {
        let _guard = self.write_lock();
        let note = self.read(id)?;
        if fingerprint(&note.type_name, &note.doc) != hash {
            return Ok(false);
        }
        if status != "released"
            && self
                .agent_safety_doc(id, &note.type_name, &note.doc)?
                .status
                == "released"
        {
            return Ok(false);
        }
        let safety = Safety {
            status: status.into(),
            reason: reason.into(),
            fingerprint: hash.into(),
        };
        self.state().set_agent_safety_result(
            id,
            &serde_json::to_string(&safety).expect("safety"),
            safety.allowed(),
        )?;
        if let Some(wake) = self
            .safety_wake
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            wake();
        }
        Ok(true)
    }
}
