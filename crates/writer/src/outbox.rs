//! Who may move an Outbox item from one status to another (outbox X2, X14).
//!
//! An item is `draft`, `ready`, `sending` or `failed`. Once sent it leaves the Outbox: its copy is
//! a regular `email`, `message` or `event` note, and the item is removed for good
//! ([`Writer::remove_sent_outbox`]).
//!
//! Every write of a note of the built-in `outbox` type passes [`Writer::check_outbox`] in
//! [`Writer::prepare_put_as`], which knows the write's author and the note's current content, so
//! the rules hold for every way in: the notes API, the chat and agents, sync from the apps, and
//! scripts.
//!
//! - The user (a session or an API token) may make any change, such as `draft -> ready` (Ready)
//!   and the retry `failed -> ready`.
//! - An agent may create an item only as `draft`, or as `ready` when its
//!   `permissions.outbox.send_without_review` is on. It may change an item only while the item
//!   is `draft` and only when it drafted it, and the item stays `draft`. It never writes
//!   `sent_ref`, `result`, `error` or `sent_at`. Its confirmation switches do not change this.
//! - A handler or a scheduled task (the send handlers and the syncs) may move an item
//!   `ready -> sending` and `sending -> failed`, write `sent_ref`, `result`, `error` and
//!   `sent_at`, and remove a `sending` item once it was sent, and nothing else. It creates no
//!   items.
//!
//! An item an agent or the user creates without `drafted_by` gets the agent's name or `user`; an
//! agent's item without a `status` is `draft`.
//!
//! Notifications (outbox X10), made once a write is committed:
//!
//! - An agent drafting items gets one notification per turn, "<Agent> drafted <n> items",
//!   linking to the Outbox's Draft tab. The items of one turn share it: each one more raises the
//!   count in place while the notification is unread, until the turn ends
//!   ([`Writer::end_outbox_drafts`]) or the agent drafts nothing for [`DRAFT_BATCH_MS`].
//! - An item a handler, a task or a device (an API token) moves to `failed` gets "Could not send <title>",
//!   with the error, linking to the item.

use schema::{Doc, Invalid, Issue, TypeDef, Value};
use state::{Author, NewNotification};

use std::collections::HashMap;

use crate::{Error, Event, Input, NoteChange, Result, Saved, Writer, first, note_path};

/// Who drafted an item the user made.
pub const DRAFTED_BY_USER: &str = "user";

/// The fields the send handlers and the syncs write.
pub const SEND_FIELDS: [&str; 4] = ["sent_ref", "result", "error", "sent_at"];

/// The fields every write may change, which the writer keeps itself.
const KEPT: [&str; 3] = ["created", "updated", "archived"];

/// The status transitions a handler or a task may make.
const SEND_TRANSITIONS: [(&str, &str); 2] = [("ready", "sending"), ("sending", "failed")];

/// Where the Outbox lists drafts: the drafts notification links there.
pub const DRAFT_TAB_URL: &str = "/outbox?tab=draft";

/// How long a sent item whose copy has not come back stays in the Outbox before the sweep
/// removes it anyway ([`Writer::sweep_outbox`]), in milliseconds: 24 hours.
pub const SENT_KEEP_MS: i64 = 24 * 60 * 60 * 1000;

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

/// How long an agent's drafts notification stays open for more of the same turn's items once the
/// agent stops drafting, in milliseconds: ten minutes.
pub const DRAFT_BATCH_MS: i64 = 10 * 60 * 1000;

/// An agent's open notification of the items it drafted this turn.
#[derive(Debug, Clone)]
pub(crate) struct DraftBatch {
    notification: String,
    count: usize,
    last_ms: i64,
}

/// One committed write of an Outbox item: its status before and after, for the notifications.
#[derive(Debug, Clone)]
pub(crate) struct ItemChange {
    id: String,
    author: Author,
    title: String,
    was: Option<String>,
    status: String,
    error: String,
    /// `email`, `sms` or `event`.
    kind: String,
    /// Who it goes to: addresses or a number, never the body.
    recipients: Vec<String>,
    /// Whether this write recorded the send: `sent_at` set where it was empty.
    sent_now: bool,
}

/// The strings of list field `name`.
fn strings(doc: &Doc, name: &str) -> Vec<String> {
    match doc.field(name) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::Str(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

impl ItemChange {
    pub(crate) fn new(id: &str, author: &Author, old: Option<&Doc>, doc: &Doc) -> ItemChange {
        ItemChange {
            id: id.to_owned(),
            author: author.clone(),
            title: text(doc, "title").unwrap_or("").trim().to_owned(),
            was: old.map(|o| text(o, "status").unwrap_or("").to_owned()),
            status: text(doc, "status").unwrap_or("").to_owned(),
            error: text(doc, "error").unwrap_or("").trim().to_owned(),
            kind: text(doc, "kind").unwrap_or("email").to_owned(),
            recipients: match text(doc, "kind").unwrap_or("email") {
                "sms" => strings(doc, "phone"),
                "event" => strings(doc, "attendees_raw"),
                _ => strings(doc, "to"),
            },
            sent_now: text(doc, "sent_at").is_some_and(|s| !s.is_empty())
                && old
                    .and_then(|o| text(o, "sent_at"))
                    .is_none_or(str::is_empty),
        }
    }

    /// Whether the write put the item into `status`.
    fn entered(&self, status: &str) -> bool {
        self.status == status && self.was.as_deref() != Some(status)
    }
}

/// `s` cut to at most `max` characters.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// "<label> drafted <n> item(s)".
fn drafted_title(label: &str, count: usize) -> String {
    let items = if count == 1 { "item" } else { "items" };
    clip(
        &format!("{label} drafted {count} {items}"),
        state::MAX_TITLE,
    )
}

impl Writer {
    /// The bell notifications for committed Outbox writes (outbox X10). A notification that
    /// cannot be stored is logged, never failing the write.
    pub(crate) fn outbox_notifications(&self, items: &[ItemChange]) {
        for item in items {
            self.record_outbox(item);
        }
        for item in items {
            match &item.author {
                Author::Agent(name) if item.entered("draft") => self.count_draft(name),
                // A send handler, a sync task, or a device reporting its command's result.
                Author::Handler(_) | Author::Task(_) | Author::Token(_)
                    if item.entered("failed") =>
                {
                    self.notify_failed(item);
                }
                _ => {}
            }
        }
    }

    /// The activity entry of an Outbox item's new status (activity Y2): drafted, ready, sending,
    /// sent (the write that records `sent_at`) or failed, with its kind and recipients but not
    /// its body.
    fn record_outbox(&self, item: &ItemChange) {
        let (act, verb) = if item.sent_now && item.status == "sending" {
            ("sent", "Sent")
        } else if item.was.as_deref() == Some(item.status.as_str()) {
            return;
        } else {
            match item.status.as_str() {
                "draft" if item.was.is_none() => ("drafted", "Drafted"),
                "draft" => return,
                "ready" => ("ready", "Ready to send"),
                "sending" => ("sending", "Sending"),
                "failed" => ("failed", "Could not send"),
                _ => return,
            }
        };
        let to = if item.recipients.is_empty() {
            String::new()
        } else {
            let shown: Vec<&str> = item.recipients.iter().take(3).map(String::as_str).collect();
            let more = item.recipients.len().saturating_sub(3);
            let more = if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            };
            format!(" to {}{more}", shown.join(", "))
        };
        let summary = format!(
            "{verb} {} {}{to}",
            item.kind,
            crate::activity::quoted(Some(&item.title))
        );
        let mut detail = serde_json::json!({
            "kind": item.kind,
            "status": item.status,
            "was": item.was,
            "to": item.recipients.iter().take(20).collect::<Vec<_>>(),
        });
        if item.status == "failed" && !item.error.is_empty() {
            detail["error"] = state::one_line(&item.error, 300).into();
        }
        self.state.record_activity(
            state::NewActivity::new(format!("outbox.{act}"), "outbox", &item.author, summary)
                .subject("outbox", &item.id)
                .detail(detail),
        );
    }

    /// End agent `name`'s turn: the next item it drafts starts a new notification. Agent runs and
    /// chat turns call this as they end.
    pub fn end_outbox_drafts(&self, name: &str) {
        self.drafts_slot().remove(name);
    }

    fn drafts_slot(&self) -> std::sync::MutexGuard<'_, HashMap<String, DraftBatch>> {
        self.drafts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One more item agent `name` drafted into Review this turn.
    fn count_draft(&self, name: &str) {
        let now = self.now_ms();
        let label = self
            .agent(name)
            .map_or_else(|_| name.to_owned(), |a| a.label);
        let mut drafts = self.drafts_slot();
        if let Some(batch) = drafts
            .get_mut(name)
            .filter(|b| now.saturating_sub(b.last_ms) <= DRAFT_BATCH_MS)
        {
            let count = batch.count + 1;
            match self.state().update_notification(
                &batch.notification,
                &drafted_title(&label, count),
                None,
            ) {
                Ok(Some(_)) => {
                    batch.count = count;
                    batch.last_ms = now;
                    return;
                }
                // Read or dismissed since: the next items start a notification of their own.
                Ok(None) => {}
                Err(e) => eprintln!("outbox: updating {name}'s drafts notification: {e}"),
            }
        }
        let n = NewNotification {
            title: drafted_title(&label, 1),
            body: Some("They wait in the Outbox's Draft tab until you mark them Ready.".to_owned()),
            note_id: None,
            url: Some(DRAFT_TAB_URL.to_owned()),
            source: format!("agent:{name}"),
        };
        match self.state().add_notification(&n) {
            Ok(made) => {
                drafts.insert(
                    name.to_owned(),
                    DraftBatch {
                        notification: made.id,
                        count: 1,
                        last_ms: now,
                    },
                );
            }
            Err(e) => eprintln!("outbox: notifying {name}'s drafts: {e}"),
        }
    }

    /// "Could not send <title>", linking to the item.
    fn notify_failed(&self, item: &ItemChange) {
        let title = if item.title.is_empty() {
            "Untitled"
        } else {
            &item.title
        };
        let n = NewNotification {
            title: clip(&format!("Could not send {title}"), state::MAX_TITLE),
            body: Some(clip(&item.error, state::MAX_BODY)).filter(|b| !b.is_empty()),
            note_id: Some(item.id.clone()),
            url: None,
            source: "outbox".to_owned(),
        };
        if let Err(e) = self.state().add_notification(&n) {
            eprintln!("outbox: notifying the failed send of {}: {e}", item.id);
        }
    }

    /// Fill in what an item's author leaves out, before the type's defaults: `drafted_by` for the
    /// user's and an agent's new items, and `status = draft` for an agent's.
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
            set(doc, "status", "draft");
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

    /// Remove Outbox item `id`, which was sent, for good (outbox X14): it is deleted and purged
    /// from the trash with its history, authored as `author`, the send handler, a sync or the
    /// server. Its copy is a regular note, and the activity log keeps the `outbox.sent` entry
    /// the send recorded. This is the server's own lifecycle, not the user emptying the trash.
    ///
    /// Only an item still `sending` is removed (or, for the upgrade from the old statuses, one
    /// that is `sent`); `base`, when given, is the version it must be at. An item already gone
    /// is [`Error::NotFound`].
    pub fn remove_sent_outbox(
        &self,
        id: &str,
        base: Option<&str>,
        author: &Author,
    ) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let (type_name, old) = self.existing(id)?;
        let path = note_path(&type_name, id);
        let built_in = type_name == crate::builtin::OUTBOX_TYPE
            && inner
                .schema
                .type_def(&type_name)
                .is_some_and(|t| self.is_built_in_def(t));
        let doc = Input::Wcl(old.clone()).parse(&path)?;
        let status = text(&doc, "status").unwrap_or("");
        if !built_in || !matches!(status, "sending" | "sent") {
            return Err(Error::Invalid(Invalid {
                file: path,
                issues: vec![Issue::new(
                    "status",
                    format!(
                        "only an Outbox item that was sent is removed, and this one is {status:?}"
                    ),
                )],
            }));
        }
        if let Some(base) = base {
            self.check_base(id, &type_name, &old, Some(base))?;
        }
        let change = NoteChange {
            id: id.to_owned(),
            type_name,
            event: Event::Delete,
            doc: None,
        };
        let saved = self
            .apply(inner, vec![change], &|| Ok(()), author)
            .map(first)?;
        if let Some(entry) = self.state.trash_entry(id)? {
            self.purge_locked(vec![entry], author)?;
        }
        Ok(saved)
    }

    /// Remove every sent Outbox item still waiting for its copy after [`SENT_KEEP_MS`] (outbox
    /// X14): `sending` with `sent_ref` and `sent_at` set, sent that long before `now_ms`, and an
    /// item whose copy is linked (`result`) that a handler did not remove. The home server's
    /// scheduler calls this as it ticks. Answers how many items it removed; one that cannot be
    /// removed is logged and left for the next sweep.
    pub fn sweep_outbox(&self, now_ms: i64) -> usize {
        use script_core::Vault as _;
        let rows = match self.query(
            &format!(
                "from {} include archived where status == \"sending\"",
                crate::builtin::OUTBOX_TYPE
            ),
            10_000,
        ) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("outbox: the sweep could not read the Outbox: {e}");
                return 0;
            }
        };
        let author = Author::Task(crate::shipped::SERVER_AUTHOR.to_owned());
        let mut removed = 0;
        for row in rows {
            let fields = &row["fields"];
            let field = |k: &str| fields[k].as_str().unwrap_or("");
            let Some(sent) = schema::parse_datetime(field("sent_at")) else {
                continue;
            };
            let sent_ms = (sent * 1000.0) as i64;
            let linked = !field("result").is_empty();
            let stale = !field("sent_ref").is_empty() && now_ms - sent_ms >= SENT_KEEP_MS;
            if !linked && !stale {
                continue;
            }
            let Some(id) = row["id"].as_str() else {
                continue;
            };
            match self.remove_sent_outbox(id, None, &author) {
                Ok(_) => removed += 1,
                Err(Error::NotFound(_)) => {}
                Err(e) => eprintln!("outbox: the sweep could not remove sent item {id}: {e}"),
            }
        }
        removed
    }

    /// Whether agent `name` has `permissions.outbox.send_without_review` on. An agent whose file
    /// is gone or does not parse has not.
    fn may_send_without_review(&self, name: &str) -> bool {
        self.agent(name)
            .is_ok_and(|a| a.permissions.outbox.send_without_review)
    }
}

/// The X2 rule for one write, as the message of the issue that refuses it (outbox X14).
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
                        "draft" => {}
                        "ready" if bypass(name) => {}
                        "ready" => {
                            return Err(format!(
                                "{label} may not send without review: its items start as draft \
                                 until the user marks them ready (turn on Send from the Outbox \
                                 without review for the agent to allow it)"
                            ));
                        }
                        other => {
                            return Err(format!(
                                "{label} creates Outbox items as draft, not {other:?}"
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
                    if was != "draft" {
                        return Err(format!(
                            "{label} may change an Outbox item only while it is draft, and this \
                             one is {was}"
                        ));
                    }
                    if text(old, "drafted_by") != Some(name.as_str()) {
                        return Err(format!(
                            "{label} may change only the Outbox items it drafted"
                        ));
                    }
                    if status != "draft" {
                        return Err(format!(
                            "{label} may not set an Outbox item to {status}: only the user marks \
                             an item ready"
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
                    "{label} may move an Outbox item only from ready to sending, and from \
                     sending to failed, not from {was} to {status}"
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
