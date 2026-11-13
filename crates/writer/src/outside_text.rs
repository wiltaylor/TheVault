//! Action policy for outside-text runs, independent of the current handler author.

use schema::{Doc, Value};
use state::Author;

use crate::{Error, Event, Input, NoteChange, Result, Writer, note_path};

fn field<'a>(doc: Option<&'a Doc>, name: &str) -> Option<&'a Value> {
    doc.and_then(|d| d.field(name))
}

fn text<'a>(doc: Option<&'a Doc>, name: &str) -> Option<&'a str> {
    field(doc, name).and_then(Value::as_str)
}

/// Shared with proposal selection. This classifies the actual typed change, not
/// a model's claimed diff. Existing permissions remain an additional upper bound.
pub fn outside_text_sensitive(
    kind: &str,
    event: Event,
    old: Option<&Doc>,
    next: Option<&Doc>,
    author: &Author,
) -> bool {
    if event == Event::Delete {
        return true;
    }
    if kind == "outbox" {
        return matches!(text(next, "status"), Some("ready" | "sending"));
    }
    if kind == "email" {
        // TV-243 local processed-field rules still work. Only the authors whose
        // changes can invoke Gmail actions need review for this transition.
        return matches!(
            author,
            Author::Session(_) | Author::Token(_) | Author::AgentMail(_)
        ) && text(next, "gmail_ref").is_some_and(|s| !s.is_empty())
            && matches!(field(next, "processed"), Some(Value::Bool(true)))
            && !matches!(field(old, "processed"), Some(Value::Bool(true)));
    }
    if matches!(kind, "event" | "event_series") {
        let google = [old, next]
            .into_iter()
            .any(|doc| text(doc, "google_ref").is_some_and(|s| !s.is_empty()));
        return (google && old != next)
            || [
                "attendees",
                "attendees_raw",
                "my_response",
                "status",
                "send_updates",
                "google_ref",
                "etag",
            ]
            .into_iter()
            .any(|name| field(old, name) != field(next, name))
            || old
                .into_iter()
                .flat_map(|d| &d.blocks)
                .filter(|b| b.kind == "dict" && b.label() == Some("snapshot"))
                .collect::<Vec<_>>()
                != next
                    .into_iter()
                    .flat_map(|d| &d.blocks)
                    .filter(|b| b.kind == "dict" && b.label() == Some("snapshot"))
                    .collect::<Vec<_>>();
    }
    false
}

impl Writer {
    pub(crate) fn outside_text_restricted(&self) -> bool {
        script_core::RunContext::current().is_some_and(|c| c.is_restricted())
    }

    pub(crate) fn record_outside_text_refusal(&self, author: &Author, action: &str) {
        self.state().record_activity(
            state::NewActivity::new(
                "run.outside_text_refused",
                "automation",
                author,
                script_core::OUTSIDE_TEXT_REFUSAL,
            )
            .detail(serde_json::json!({"action": action})),
        );
    }

    pub(crate) fn hold_outside_text_outbox(&self, change: &mut NoteChange, author: &Author) {
        if change.type_name == "outbox"
            && let Some(doc) = change.doc.as_mut()
        {
            self.hold_outside_text_ready(doc, author);
        }
    }

    pub(crate) fn hold_outside_text_ready(&self, doc: &mut Doc, author: &Author) {
        if self.outside_text_restricted() && text(Some(doc), "status") == Some("ready") {
            doc.fields.retain(|(name, _)| name != "status");
            doc.fields
                .push(("status".into(), Value::Str("draft".into())));
            self.record_outside_text_refusal(author, "outbox_ready");
        }
    }

    pub(crate) fn check_outside_text_write(
        &self,
        change: &NoteChange,
        author: &Author,
    ) -> Result<()> {
        if !self.outside_text_restricted() {
            return Ok(());
        }
        let old = if change.event == Event::Create || change.event == Event::Delete {
            None
        } else {
            let (_, text) = self.existing(&change.id)?;
            Some(Input::Wcl(text).parse(&note_path(&change.type_name, &change.id))?)
        };
        if outside_text_sensitive(
            &change.type_name,
            change.event,
            old.as_ref(),
            change.doc.as_ref(),
            author,
        ) {
            self.record_outside_text_refusal(author, "note_mutation");
            return Err(Error::Rejected(script_core::OUTSIDE_TEXT_REFUSAL.into()));
        }
        Ok(())
    }
}
