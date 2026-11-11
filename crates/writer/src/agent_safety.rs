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
        "email_thread" | "conversation" => true,
        "email" => !fields["labels"].as_array().is_some_and(|labels| {
            labels
                .iter()
                .any(|l| matches!(l.as_str(), Some("system/SENT" | "system/DRAFT")))
        }),
        "message" => fields["direction"] != "out",
        "call" => fields["direction"] != "out",
        // Without account state an organiser cannot establish ownership.
        "event" | "event_series" => {
            if fields["organiser"].as_str().is_some_and(|s| !s.is_empty()) {
                true
            } else {
                fields.get("google_ref").is_some_and(|v| !v.is_null())
            }
        }
        _ => false,
    }
}

/// The same content projection is sent to Jev and hashed. Workflow and sync bookkeeping
/// are not evidence. Unknown/custom fields remain evidence rather than silently bypassing it.
pub fn classified_content(kind: &str, doc: &Doc) -> serde_json::Value {
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
        if matches!(kind, "email_thread" | "conversation") {
            // Server person references change as members arrive, not the derived title.
            fields.remove("participants");
        }
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
        Sha256::digest(format!("{kind}:{}", classified_content(kind, doc)))
    )
}

// Only image rendering is reduced. Paragraph text, image alt text and captions
// remain evidence, so replacing a plain-text import with different HTML is held.
fn same_image_rendering(old: &serde_json::Value, next: &serde_json::Value) -> bool {
    let mut old = old.clone();
    let mut next = next.clone();
    if old["kind"] == "paragraph" && next["kind"] == "image" {
        let Some((alt, _)) = old["fields"]["text"]
            .as_str()
            .and_then(html_blocks::pending_image)
        else {
            return false;
        };
        // Only the exact placeholder produced by the converter may become an image.
        if old["fields"]
            .as_object()
            .is_none_or(|fields| fields.len() != 1)
        {
            return false;
        }
        old["kind"] = serde_json::json!("image");
        old["fields"] = serde_json::json!({});
        if !alt.is_empty() {
            old["fields"]["alt"] = serde_json::json!(alt);
        }
    }
    if old["kind"] == "image" && next["kind"] == "image" {
        for value in [&mut old, &mut next] {
            if let Some(fields) = value["fields"].as_object_mut() {
                fields.remove("asset");
            }
        }
    }
    match (old.get("blocks"), next.get("blocks")) {
        (Some(a), Some(b)) => {
            let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else {
                return false;
            };
            if a.len() != b.len() || !a.iter().zip(b).all(|(a, b)| same_image_rendering(a, b)) {
                return false;
            }
            old.as_object_mut().unwrap().remove("blocks");
            next.as_object_mut().unwrap().remove("blocks");
        }
        (None, None) => {}
        _ => return false,
    }
    old == next
}

impl Writer {
    pub fn incoming_agent_note(&self, kind: &str, doc: &Doc) -> bool {
        if matches!(kind, "event" | "event_series")
            && let Some(organiser) = doc.field("organiser").and_then(schema::Value::as_str)
            && !organiser.is_empty()
            && let Ok(Some(account)) = self.state().get_secret(crate::google::ACCOUNT)
            && organiser.eq_ignore_ascii_case(account.trim())
        {
            return false;
        }
        incoming(kind, doc)
    }
    /// The only incremental note-index write boundary. Safety is refreshed before
    /// publishing notes, including imports which bypass normal write preparation.
    pub(crate) fn update_note_index(
        &self,
        inner: &mut crate::Inner,
        updates: &[index::Update],
    ) -> Result<()> {
        for update in updates {
            match update {
                index::Update::Put {
                    id, type_name, doc, ..
                } => {
                    self.bind_safety_member(id, type_name, doc)?;
                    self.refresh_agent_safety_index(id, type_name, doc)?;
                }
                index::Update::Delete { id } => self.state().forget_agent_safety_index(id)?,
            }
        }
        inner.index.update(&inner.schema, updates)?;
        for update in updates {
            let (index::Update::Put { id, .. } | index::Update::Delete { id }) = update;
            if matches!(update, index::Update::Delete { .. })
                && self.state().meta("agent_safety.activation_ms")?.is_some()
            {
                let parents = self.state().agent_safety_parents(id)?;
                self.state().unbind_deleted_safety_member(id)?;
                for parent in &parents {
                    use rusqlite::OptionalExtension;
                    let next: Option<String> = inner.index.connection().query_row(
                        "SELECT n.id FROM field_values f JOIN notes n ON n.id=f.note WHERE f.str=?1 AND ((f.field='thread' AND n.type='email') OR (f.field='conversation' AND n.type='message')) ORDER BY n.created_at,n.id LIMIT 1",
                        [parent], |r| r.get(0),
                    ).optional().map_err(index::Error::from)?;
                    if let Some(next) = next {
                        self.state().bind_agent_safety_member(parent, &next)?;
                        self.refresh_safety_parents(&next)?;
                    } else {
                        self.state()
                            .set_meta(&format!("agent_safety.no_members.{parent}"), "1")?;
                        if let Some(raw) =
                            self.state().meta(&format!("agent_safety.note.{parent}"))?
                            && let Ok(safety) = serde_json::from_str::<Safety>(&raw)
                            && self.state().indexed_agent_safety_hash(parent)?.as_deref()
                                == Some(&safety.fingerprint)
                        {
                            self.state()
                                .set_agent_safety_result(parent, &raw, safety.allowed())?;
                        }
                    }
                }
            }
            self.refresh_safety_parents(id)?;
        }
        Ok(())
    }

    /// Gmail messages are immutable by Gmail id. Only the trusted sync task may carry a
    /// decision across a local rendering change of that same message. This is not an
    /// editable note flag and is never available to an agent/session write.
    pub(crate) fn carry_safety_conversion(
        &self,
        id: &str,
        kind: &str,
        next: &Doc,
        actor: &state::Author,
    ) -> Result<()> {
        if kind != "email" || !matches!(actor, state::Author::Task(name) if name == "google_gmail")
        {
            return Ok(());
        }
        let Some(old) = self.state().head(id)? else {
            return Ok(());
        };
        let Ok(old) = Doc::parse(&old.content, "previous email") else {
            return Ok(());
        };
        let source = old.field("gmail_ref").and_then(schema::Value::as_str);
        if source.is_none_or(str::is_empty)
            || source != next.field("gmail_ref").and_then(schema::Value::as_str)
        {
            return Ok(());
        }
        let without_rendering = |doc: &Doc| {
            let mut value = classified_content(kind, doc);
            if let Some(fields) = value["fields"].as_object_mut() {
                fields.remove("html");
                fields.remove("attachments");
            }
            value
        };
        if !same_image_rendering(&without_rendering(&old), &without_rendering(next)) {
            return Ok(());
        }
        let before = fingerprint(kind, &old);
        let after = fingerprint(kind, next);
        if before == after {
            return Ok(());
        }
        let Some(raw) = self.state().meta(&format!("agent_safety.note.{id}"))? else {
            return Ok(());
        };
        let Ok(mut safety) = serde_json::from_str::<Safety>(&raw) else {
            return Ok(());
        };
        if safety.fingerprint != before {
            return Ok(());
        }
        self.state()
            .set_meta(&format!("agent_safety.conversion.{id}.{before}"), &after)?;
        safety.fingerprint = after;
        // Preserve held/pending as well as clearance. No new arrival is emitted here.
        self.state().set_agent_safety_result(
            id,
            &serde_json::to_string(&safety).expect("safety"),
            safety.allowed(),
        )?;
        Ok(())
    }
    fn safety_member_cleared(&self, id: &str) -> Result<bool> {
        let Some(member) = self.state().meta(&format!("agent_safety.member.{id}"))? else {
            return Ok(self
                .state()
                .meta(&format!("agent_safety.no_members.{id}"))?
                .is_some());
        };
        Ok(self.state().indexed_agent_note_allowed(&member)?)
    }

    /// The first committed member is pinned in server state, not editable reference fields.
    pub(crate) fn bind_safety_member(&self, id: &str, kind: &str, doc: &Doc) -> Result<()> {
        if self.state().meta("agent_safety.activation_ms")?.is_none() {
            return Ok(());
        }
        let field = match kind {
            "email" => "thread",
            "message" => "conversation",
            _ => return Ok(()),
        };
        if let Some(parent) = doc.field(field).and_then(schema::Value::as_str) {
            self.state().bind_agent_safety_member(parent, id)?;
        }
        Ok(())
    }

    pub(crate) fn refresh_safety_parents(&self, member: &str) -> Result<()> {
        if self.state().meta("agent_safety.activation_ms")?.is_none() {
            return Ok(());
        }
        for parent in self.state().agent_safety_parents(member)? {
            let Some(raw) = self.state().meta(&format!("agent_safety.note.{parent}"))? else {
                continue;
            };
            let Ok(safety) = serde_json::from_str::<Safety>(&raw) else {
                continue;
            };
            let Some(hash) = self.state().indexed_agent_safety_hash(&parent)? else {
                continue;
            };
            let allowed = safety.fingerprint == hash
                && safety.allowed()
                && (safety.status == "released" || self.safety_member_cleared(&parent)?);
            self.state().index_agent_safety(&parent, &hash, !allowed)?;
            if allowed {
                self.state().set_agent_safety_result(&parent, &raw, true)?;
            }
        }
        if self.state().indexed_agent_safety_hash(member)?.is_none() {
            self.state().unbind_deleted_safety_member(member)?;
        }
        Ok(())
    }
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
        since.map(parse).transpose()?;
        until.map(parse).transpose()?;
        let held = self.state().held_agent_note_ids()?;
        let held_json = serde_json::to_string(&held).expect("held ids");
        let notes = self
            .read_index(|index, _| {
                let mut stmt = index.connection().prepare(
                    "SELECT n.id,n.type,n.created FROM json_each(?1) h JOIN notes n ON n.id=h.value
                        WHERE (?2 IS NULL OR n.type=?2)
                        AND (?3 IS NULL OR julianday(n.created)>=julianday(?3))
                        AND (?4 IS NULL OR julianday(n.created)<=julianday(?4))",
                )?;
                stmt.query_map(rusqlite::params![held_json, kind, since, until], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        (r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?),
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(index::Error::from)?;
        let mut out = Vec::new();
        for (id, (type_name, created)) in notes {
            let Some(hash) = self.state().indexed_agent_safety_hash(&id)? else {
                continue;
            };
            let stored = self.state().meta(&format!("agent_safety.note.{id}"))?;
            let mut safety = stored
                .and_then(|v| serde_json::from_str::<Safety>(&v).ok())
                .unwrap_or(Safety {
                    status: "held".into(),
                    reason: "unclassified".into(),
                    fingerprint: hash.clone(),
                });
            if safety.fingerprint != hash {
                safety = Safety {
                    status: "held".into(),
                    reason: "content_changed".into(),
                    fingerprint: hash,
                };
            }
            if safety.status == "allowed"
                && matches!(type_name.as_str(), "email_thread" | "conversation")
                && !self.safety_member_cleared(&id)?
            {
                safety.status = "held".into();
                safety.reason = "awaiting_first_member".into();
            }
            if !safety.allowed() && reason.is_none_or(|r| r == safety.reason) {
                out.push(serde_json::json!({"id":id,"type":type_name,"created":created,"agent_safety":safety}));
            }
        }
        Ok(out)
    }

    pub fn activate_agent_safety(&self) -> Result<()> {
        self.state().prepare_agent_safety_parents()?;
        if self.state().meta("agent_safety.assets_migrated")?.is_none() {
            for note in self.list_with(None, Archived::Include)? {
                let note = self.read(&note.id)?;
                self.mark_incoming_note_assets(&note.id, &note.type_name, &note.doc)?;
            }
            for entry in self.state().list_trash()? {
                if let Ok(doc) = Doc::parse(&entry.content, "trashed.wcl") {
                    self.mark_incoming_note_assets(&entry.note_id, &entry.note_type, &doc)?;
                }
            }
            self.state().set_meta("agent_safety.assets_migrated", "1")?;
        }
        if self.state().meta("agent_safety.index_ready")?.as_deref() == Some("1") {
            for (id, _, stored) in self.state().indexed_agent_safety()? {
                if let Some(mut safety) =
                    stored.and_then(|v| serde_json::from_str::<Safety>(&v).ok())
                    && safety.status == "pending"
                    && !self.state().classification_queued(&id)?
                {
                    safety.status = "held".into();
                    safety.reason = "unclassified".into();
                    self.state().set_agent_safety_result(
                        &id,
                        &serde_json::to_string(&safety).expect("safety"),
                        false,
                    )?;
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
                if self.incoming_agent_note(&note.type_name, &note.doc) {
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
        if stored.is_none() && !self.incoming_agent_note(kind, doc) {
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
                    if matches!(kind, "email_thread" | "conversation")
                        && safety.status == "allowed"
                        && !self.safety_member_cleared(id)?
                    {
                        return Ok(result("held", "awaiting_first_member"));
                    }
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
        self.mark_incoming_note_assets(id, kind, doc)?;
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
        if !self
            .state()
            .meta(&format!("agent_safety.asset.{id}"))
            .is_ok_and(|v| v.is_none())
        {
            return false;
        }
        self.asset_usage(id).is_ok_and(|usage| {
            usage.notes.iter().all(|n| {
                self.agent_may_read(&n.note)
                    && self.read(&n.note).is_ok_and(|note| {
                        !self.incoming_agent_note(&note.type_name, &note.doc)
                            && self
                                .state()
                                .meta(&format!("agent_safety.note.{}", n.note))
                                .is_ok_and(|v| v.is_none())
                            && self
                                .state()
                                .meta(&format!("agent_safety.legacy.{}", n.note))
                                .is_ok_and(|v| v.is_none())
                    })
            }) && usage.trashed.is_empty()
        })
    }

    /// Sticky provenance. Unlinking, deleting or purging a note never releases its files.
    pub fn mark_incoming_asset(&self, id: &str) -> Result<()> {
        self.state()
            .set_meta(&format!("agent_safety.asset.{id}"), "1")?;
        Ok(())
    }

    fn mark_incoming_note_assets(&self, id: &str, kind: &str, doc: &Doc) -> Result<()> {
        if self.incoming_agent_note(kind, doc)
            || self
                .state()
                .meta(&format!("agent_safety.note.{id}"))?
                .is_some()
            || self
                .state()
                .meta(&format!("agent_safety.legacy.{id}"))?
                .is_some()
        {
            fn walk(w: &Writer, value: &serde_json::Value) -> Result<()> {
                match value {
                    serde_json::Value::String(s) if schema::is_ulid(s) => {
                        w.mark_incoming_asset(s)?
                    }
                    serde_json::Value::Array(a) => {
                        for v in a {
                            walk(w, v)?;
                        }
                    }
                    serde_json::Value::Object(o) => {
                        for v in o.values() {
                            walk(w, v)?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            walk(self, &crate::doc_to_json(doc))?;
        }
        Ok(())
    }

    /// Called under the existing commit lock by the after-write hook.
    pub fn register_agent_safety(&self, id: &str, kind: &str, doc: &Doc) -> Result<()> {
        if self.incoming_agent_note(kind, doc)
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
        let current = fingerprint(&note.type_name, &note.doc);
        let mut equivalent = hash.to_owned();
        // Bound corrupt state/chains. Only the trusted conversion path records these edges.
        for _ in 0..32 {
            if equivalent == current {
                break;
            }
            let Some(next) = self
                .state()
                .meta(&format!("agent_safety.conversion.{id}.{equivalent}"))?
            else {
                break;
            };
            equivalent = next;
        }
        if current != equivalent || (status == "released" && current != hash) {
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
            fingerprint: current,
        };
        self.state().set_agent_safety_result(
            id,
            &serde_json::to_string(&safety).expect("safety"),
            safety.allowed()
                && (status == "released"
                    || !matches!(note.type_name.as_str(), "email_thread" | "conversation")
                    || self.safety_member_cleared(id)?),
        )?;
        self.refresh_safety_parents(id)?;
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

#[cfg(test)]
mod rendering_tests {
    use super::same_image_rendering;
    use serde_json::json;

    #[test]
    fn image_fields_other_than_asset_remain_evidence() {
        let a = json!({"kind":"image","fields":{"asset":"a","alt":"alt","caption":"caption","width":100}});
        let mut b = a.clone();
        b["fields"]["asset"] = json!("b");
        assert!(same_image_rendering(&a, &b));
        for field in ["alt", "caption", "width", "custom"] {
            let mut changed = b.clone();
            changed["fields"][field] = json!("changed");
            assert!(!same_image_rendering(&a, &changed), "{field}");
        }
        let paragraph = json!({"kind":"paragraph","fields":{"text":"image"}});
        assert!(!same_image_rendering(&a, &paragraph));
    }
}
