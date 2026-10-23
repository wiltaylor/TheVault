//! Who may move an Outbox item from one status to another (outbox X2).
//!
//! Every write of a note of the built-in `outbox` type passes [`Writer::check_outbox`] in
//! [`Writer::prepare_put_as`], which knows the write's author and the note's current content, so
//! the rules hold for every way in: the notes API, the chat and agents, sync from the apps, and
//! scripts.
//!
//! - The user (a session or an API token) may make any change, such as `draft -> approved`
//!   (Send), `review -> approved | rejected` and the retry `failed -> approved`.
//! - An agent may create an item only as `review`, or as `approved` when its
//!   `permissions.outbox.send_without_review` is on. It may change an item only while the item
//!   is `draft` or `review` and only when it drafted it, and only to `draft` or `review`. It never
//!   writes `sent_ref`, `result`, `error` or `sent_at`. Its confirmation switches do not change
//!   this.
//! - A handler or a scheduled task (the send handlers and the syncs) may move an item
//!   `approved -> sending`, `sending -> sent` and `sending -> failed`, and write `sent_ref`,
//!   `result`, `error` and `sent_at`, and nothing else. It creates no items.
//!
//! An item an agent or the user creates without `drafted_by` gets the agent's name or `user`; an
//! agent's item without a `status` is `review`.

use schema::{Doc, Invalid, Issue, TypeDef, Value};
use state::Author;

use crate::{Error, Result, Writer};

/// Who drafted an item the user made.
pub const DRAFTED_BY_USER: &str = "user";

/// The fields the send handlers and the syncs write.
pub const SEND_FIELDS: [&str; 4] = ["sent_ref", "result", "error", "sent_at"];

/// The fields every write may change, which the writer keeps itself.
const KEPT: [&str; 3] = ["created", "updated", "archived"];

/// The status transitions a handler or a task may make.
const SEND_TRANSITIONS: [(&str, &str); 3] = [
    ("approved", "sending"),
    ("sending", "sent"),
    ("sending", "failed"),
];

fn text<'a>(doc: &'a Doc, name: &str) -> Option<&'a str> {
    doc.field(name).and_then(Value::as_str)
}

fn set(doc: &mut Doc, name: &str, value: &str) {
    doc.fields.retain(|(n, _)| n != name);
    doc.fields
        .push((name.to_owned(), Value::Str(value.to_owned())));
}

/// The names of the fields whose values differ between `a` and `b`, and whether the blocks do.
fn changed<'a>(a: &'a Doc, b: &'a Doc) -> (Vec<&'a str>, bool) {
    let mut names: Vec<&str> = a
        .fields
        .iter()
        .chain(&b.fields)
        .map(|(n, _)| n.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    let fields = names
        .into_iter()
        .filter(|n| !KEPT.contains(n) && a.field(n) != b.field(n))
        .collect();
    (fields, a.blocks != b.blocks)
}

impl Writer {
    /// Fill in what an item's author leaves out, before the type's defaults: `drafted_by` for the
    /// user's and an agent's new items, and `status = review` for an agent's.
    pub(crate) fn fill_outbox(&self, doc: &mut Doc, is_new: bool, author: &Author) {
        if !is_new {
            return;
        }
        let drafted_by = match author {
            Author::Agent(name) => name.as_str(),
            Author::Session(_) | Author::Token(_) => DRAFTED_BY_USER,
            Author::Handler(_) | Author::Task(_) => return,
        };
        if text(doc, "drafted_by").is_none_or(str::is_empty) {
            set(doc, "drafted_by", drafted_by);
        }
        if matches!(author, Author::Agent(_)) && doc.field("status").is_none() {
            set(doc, "status", "review");
        }
    }

    /// Refuse a write of an Outbox item that `author` may not make (outbox X2). `doc` is the item
    /// as it will be written, with the type's defaults; `old` is its current content, `None` for
    /// a new item.
    pub(crate) fn check_outbox(
        &self,
        ty: &TypeDef,
        path: &str,
        doc: &Doc,
        old: Option<&Doc>,
        author: &Author,
    ) -> Result<()> {
        if !self.is_built_in_def(ty) {
            return Ok(());
        }
        match outbox_rule(doc, old, author, &|name| self.may_send_without_review(name)) {
            Ok(()) => Ok(()),
            Err(message) => Err(Error::Invalid(Invalid {
                file: path.to_owned(),
                issues: vec![Issue::new("status", message)],
            })),
        }
    }

    /// Whether agent `name` has `permissions.outbox.send_without_review` on. An agent whose file
    /// is gone or does not parse has not.
    fn may_send_without_review(&self, name: &str) -> bool {
        self.agent(name)
            .is_ok_and(|a| a.permissions.outbox.send_without_review)
    }
}

/// The X2 rule for one write, as the message of the issue that refuses it.
fn outbox_rule(
    doc: &Doc,
    old: Option<&Doc>,
    author: &Author,
    bypass: &dyn Fn(&str) -> bool,
) -> std::result::Result<(), String> {
    let status = text(doc, "status").unwrap_or("");
    let was = old.map(|o| text(o, "status").unwrap_or("")).unwrap_or("");
    match author {
        Author::Session(_) | Author::Token(_) => Ok(()),
        Author::Agent(name) => {
            let label = format!("agent {name}");
            match old {
                None => {
                    if text(doc, "drafted_by") != Some(name.as_str()) {
                        return Err(format!(
                            "an item {label} creates is drafted by {name}: leave drafted_by out"
                        ));
                    }
                    match status {
                        "review" => {}
                        "approved" if bypass(name) => {}
                        "approved" => {
                            return Err(format!(
                                "{label} may not send without review: its items start as review \
                                 until the user approves them (turn on Send from the Outbox \
                                 without review for the agent to allow it)"
                            ));
                        }
                        other => {
                            return Err(format!(
                                "{label} creates Outbox items as review, not {other:?}"
                            ));
                        }
                    }
                    if let Some(f) = SEND_FIELDS.iter().find(|f| doc.field(f).is_some()) {
                        return Err(format!(
                            "{label} may not write {f}: only the send handlers do"
                        ));
                    }
                    Ok(())
                }
                Some(old) => {
                    if !matches!(was, "draft" | "review") {
                        return Err(format!(
                            "{label} may change an Outbox item only while it is draft or review, \
                             and this one is {was}"
                        ));
                    }
                    if text(old, "drafted_by") != Some(name.as_str()) {
                        return Err(format!(
                            "{label} may change only the Outbox items it drafted"
                        ));
                    }
                    if !matches!(status, "draft" | "review") {
                        return Err(format!(
                            "{label} may not set an Outbox item to {status}: only the user \
                             approves or rejects an item"
                        ));
                    }
                    let (fields, _) = changed(old, doc);
                    if let Some(f) = fields
                        .iter()
                        .find(|f| **f == "drafted_by" || SEND_FIELDS.contains(f))
                    {
                        return Err(format!("{label} may not change {f} of an Outbox item"));
                    }
                    Ok(())
                }
            }
        }
        Author::Handler(name) | Author::Task(name) => {
            let label = match author {
                Author::Handler(_) => format!("handler {name}"),
                _ => format!("task {name}"),
            };
            let Some(old) = old else {
                return Err(format!(
                    "{label} may not create Outbox items: the user or an agent drafts them"
                ));
            };
            if was != status && !SEND_TRANSITIONS.contains(&(was, status)) {
                return Err(format!(
                    "{label} may move an Outbox item only from approved to sending, and from \
                     sending to sent or failed, not from {was} to {status}"
                ));
            }
            let (fields, blocks) = changed(old, doc);
            if let Some(f) = fields
                .iter()
                .find(|f| **f != "status" && !SEND_FIELDS.contains(f))
            {
                return Err(format!(
                    "{label} may change only the status, sent_ref, result, error and sent_at of \
                     an Outbox item, not {f}"
                ));
            }
            if blocks {
                return Err(format!(
                    "{label} may change only the status, sent_ref, result, error and sent_at of \
                     an Outbox item, not its content"
                ));
            }
            Ok(())
        }
    }
}
