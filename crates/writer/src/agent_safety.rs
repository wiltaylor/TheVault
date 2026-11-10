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
        _ => false,
    }
}

/// All content and fields, except server timestamps and the shipped classifier's annotations.
pub fn fingerprint(kind: &str, doc: &Doc) -> String {
    let mut note = doc_to_json(doc);
    if let Some(fields) = note["fields"].as_object_mut() {
        for key in ["updated", "created"] {
            fields.remove(key);
        }
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
    format!("{:x}", Sha256::digest(format!("{kind}:{}", note)))
}

impl Writer {
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
        let mut out = Vec::new();
        for note in self.list_with(kind, Archived::Include)? {
            let date = note
                .created
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            if since.is_some_and(|s| date.is_none_or(|d| d < s))
                || until.is_some_and(|s| date.is_none_or(|d| d > s))
            {
                continue;
            }
            let safety = self.agent_safety(&note.id)?;
            if !safety.allowed() && reason.is_none_or(|r| r == safety.reason) {
                out.push(serde_json::json!({"id":note.id,"type":note.type_name,"created":note.created,"agent_safety":safety}));
            }
        }
        Ok(out)
    }

    pub fn activate_agent_safety(&self) -> Result<()> {
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
        }
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
        Ok(result("pending", "awaiting_decision"))
    }

    pub fn agent_safety(&self, id: &str) -> Result<Safety> {
        let note = self.read(id)?;
        self.agent_safety_doc(id, &note.type_name, &note.doc)
    }

    pub fn agent_may_read(&self, id: &str) -> bool {
        self.agent_safety(id).is_ok_and(|s| s.allowed())
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
                .is_none()
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
        Ok(true)
    }
}
