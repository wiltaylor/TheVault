//! What the write path records in the activity log (activity Y2): every note change, from the
//! commit path, so the API, the chat, scripts, tasks, device sync and agents are all covered; and
//! every change and delete of a type, taxonomy, agent, task or script file.
//!
//! Definition writes take no author, so the caller names who is acting with [`acting_as`] around
//! them; without it the change is the server's own, such as a shipped default. A note write knows
//! its author and names it itself.
//!
//! One actor's edits of one note coalesce into one `note.update` entry for a minute
//! ([`state::ACTIVITY_COALESCE_MS`]), so autosave while typing does not flood the log.

use std::cell::RefCell;

use serde_json::json;
use state::{Author, NewActivity, one_line};

use crate::Writer;

thread_local! {
    static ACTING: RefCell<Vec<Author>> = const { RefCell::new(Vec::new()) };
}

/// Who is acting on this thread until it is dropped. See [`acting_as`].
#[must_use = "the actor is only set while the guard lives"]
pub struct Acting(());

impl Drop for Acting {
    fn drop(&mut self) {
        ACTING.with(|a| {
            a.borrow_mut().pop();
        });
    }
}

/// Record the definition changes made on this thread as `author`'s until the guard drops.
pub fn acting_as(author: &Author) -> Acting {
    ACTING.with(|a| a.borrow_mut().push(author.clone()));
    Acting(())
}

/// The author [`acting_as`] set on this thread, if any.
pub(crate) fn acting() -> Option<Author> {
    ACTING.with(|a| a.borrow().last().cloned())
}

/// The longest title an entry's summary repeats, in characters.
const TITLE_CHARS: usize = 120;

/// What happened to a note, for its entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteAct {
    Create,
    Update,
    Archive,
    Unarchive,
    Delete,
    Restore,
    Purge,
}

impl NoteAct {
    fn kind(self) -> &'static str {
        match self {
            NoteAct::Create => "note.create",
            NoteAct::Update => "note.update",
            NoteAct::Archive => "note.archive",
            NoteAct::Unarchive => "note.unarchive",
            NoteAct::Delete => "note.delete",
            NoteAct::Restore => "note.restore",
            NoteAct::Purge => "note.purge",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            NoteAct::Create => "Created",
            NoteAct::Update => "Edited",
            NoteAct::Archive => "Archived",
            NoteAct::Unarchive => "Unarchived",
            NoteAct::Delete => "Deleted",
            NoteAct::Restore => "Restored",
            NoteAct::Purge => "Purged",
        }
    }
}

/// `"title"`, or `(untitled)`.
pub(crate) fn quoted(title: Option<&str>) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("\"{}\"", one_line(t, TITLE_CHARS)),
        None => "(untitled)".to_owned(),
    }
}

impl Writer {
    /// Record `act` on note `id` of `type_name` by `by`.
    pub(crate) fn record_note(
        &self,
        by: &Author,
        act: NoteAct,
        id: &str,
        type_name: &str,
        title: Option<&str>,
        version: &str,
    ) {
        let tail = match act {
            NoteAct::Delete => " to the trash",
            NoteAct::Restore => " from the trash",
            NoteAct::Purge => " for good",
            _ => "",
        };
        let summary = format!("{} {type_name} {}{tail}", act.verb(), quoted(title));
        let mut entry = NewActivity::new(act.kind(), "notes", by, summary)
            .subject("note", id)
            .detail(json!({
                "type": type_name,
                "title": title.map(|t| one_line(t, TITLE_CHARS)),
                "version": version,
            }));
        if act == NoteAct::Update {
            entry = entry.coalescing();
        }
        self.state.record_activity(entry);
    }

    /// Record the definition files a commit changed: `(path, existed, exists)` each. Notes and
    /// other files are not definitions and are left out.
    pub(crate) fn record_definitions(&self, changes: &[(String, bool, bool)]) {
        let by = acting();
        for (path, existed, exists) in changes {
            let Some((kind, name)) = crate::synced_file(path) else {
                continue;
            };
            let label = match kind {
                "type" => "type",
                "taxonomy" => "taxonomy",
                "agent" => "agent",
                "task" => "task",
                "script" => "script",
                _ => continue,
            };
            let (act, verb) = match (existed, exists) {
                (_, false) => ("delete", "Deleted"),
                (false, true) => ("create", "Created"),
                (true, true) => ("update", "Changed"),
            };
            let (kind, summary) = (format!("{label}.{act}"), format!("{verb} {label} {name}"));
            let entry = match &by {
                Some(by) => NewActivity::new(kind, "definitions", by, summary),
                None => NewActivity::server(kind, "definitions", summary),
            };
            self.state.record_activity(
                entry
                    .subject(label, name)
                    .detail(json!({ "path": path, "name": name })),
            );
        }
    }
}
