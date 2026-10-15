//! Writes that come from another copy of the vault (D19, D20).
//!
//! A device syncs with the home server, which holds the master copy. Two kinds of write cross
//! between them that the normal write path cannot make:
//!
//! - On the server, [`Writer::create_with_id`] creates a note under the id a device gave it while
//!   offline, so the note keeps its id and every link to it.
//! - On the device, [`Writer::apply_remote`] and [`Writer::apply_remote_files`] write what the
//!   server's change feed says, byte for byte. The server already validated the note, ran its
//!   handlers and stamped it, so none of that runs again, and no `after_*` hook sees the change:
//!   it is not a local write to send back.

use schema::{NoteReport, Schema};
use state::Author;
use store::Change;

use crate::{Error, Event, Input, Result, Saved, Writer, feed::synced_file, first, note_path};

/// The author of the versions a device records for notes it received from the home server.
pub const REMOTE_AUTHOR: &str = "sync";

/// One note change from the home server's change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remote {
    /// The note as the server holds it: its type and canonical WCL.
    Put {
        id: String,
        type_name: String,
        wcl: String,
    },
    /// The note was deleted on the server: it goes to the trash here too.
    Delete { id: String },
}

/// The top-level folders the store commits to. Other synced folders, such as `devices/`, are
/// written straight to disk.
fn in_store_layout(path: &str) -> bool {
    path.split_once('/')
        .is_some_and(|(dir, _)| store::LAYOUT_DIRS.contains(&dir))
}

impl Writer {
    /// Create a note of `type_name` under `id`, an id the client chose: a device that created the
    /// note offline keeps its id when it syncs (D20). `id` must be a ULID no note has ever had,
    /// live, in the trash or in the history; otherwise the write fails with [`Error::IdTaken`].
    pub fn create_with_id(
        &self,
        id: &str,
        type_name: &str,
        input: &Input,
        author: &Author,
    ) -> Result<Saved> {
        if !schema::is_ulid(id) {
            return Err(Error::BadRequest(format!("{id:?} is not a note id")));
        }
        let _w = self.write_lock();
        let inner = self.lock();
        if self.locate(id)?.is_some() || !self.state.history(id)?.is_empty() {
            return Err(Error::IdTaken(id.to_owned()));
        }
        let doc = input.parse(&note_path(type_name, id))?;
        let change = self.prepare_put(&inner, id, type_name, doc, None, author)?;
        self.apply(inner, vec![change], &|| Ok(()), author)
            .map(first)
    }

    /// Write a note exactly as the home server holds it, or move it to the trash, and index it.
    /// Returns the note's local version, or `None` when a delete found nothing to delete. A put
    /// of the text the note already holds records nothing and returns the current version.
    pub fn apply_remote(&self, change: &Remote) -> Result<Option<String>> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let author = Author::Token(REMOTE_AUTHOR.to_owned());
        match change {
            Remote::Put { id, type_name, wcl } => {
                if !schema::is_ulid(id) || !schema::is_name(type_name) {
                    return Err(Error::BadRequest(format!("{type_name}/{id} is not a note")));
                }
                let path = note_path(type_name, id);
                let doc = Input::Wcl(wcl.clone()).parse(&path)?;
                let old_type = self.locate(id)?;
                if old_type.as_deref() == Some(type_name.as_str())
                    && self.vault.read_text(&path)? == *wcl
                {
                    return Ok(Some(self.head(id, type_name, wcl)?));
                }
                let mut files = Vec::new();
                if let Some(old) = old_type.as_deref().filter(|t| *t != type_name) {
                    files.push(Change::delete(note_path(old, id)));
                }
                files.push(Change::write(path, wcl.as_bytes()));
                let report = report(&inner.schema, self, type_name, &doc);
                let generation = self.commit(files)?;
                let version = self.state.record_version(id, type_name, &author, wcl)?;
                let inner = &mut *inner;
                inner.index.update(
                    &inner.schema,
                    &[index::Update::Put {
                        id: id.clone(),
                        type_name: type_name.clone(),
                        doc,
                        report,
                    }],
                )?;
                inner.index.advance(generation)?;
                self.announce(&[Saved {
                    id: id.clone(),
                    type_name: type_name.clone(),
                    event: if old_type.is_some() {
                        Event::Update
                    } else {
                        Event::Create
                    },
                    version: version.token.clone(),
                    wcl: wcl.clone(),
                    also: Vec::new(),
                }]);
                Ok(Some(version.token))
            }
            Remote::Delete { id } => {
                let Some(type_name) = self.locate(id)? else {
                    return Ok(None);
                };
                let path = note_path(&type_name, id);
                let text = self.vault.read_text(&path)?;
                self.head(id, &type_name, &text)?;
                let generation = self.commit(vec![Change::delete(path)])?;
                let entry = self.state.trash_note(id, &author)?;
                let inner = &mut *inner;
                inner
                    .index
                    .update(&inner.schema, &[index::Update::Delete { id: id.clone() }])?;
                inner.index.advance(generation)?;
                self.announce(&[Saved {
                    id: id.clone(),
                    type_name,
                    event: Event::Delete,
                    version: entry.version.clone(),
                    wcl: entry.content,
                    also: Vec::new(),
                }]);
                Ok(Some(entry.version))
            }
        }
    }

    /// Write synced files (types, taxonomies, scripts, tasks, `devices/`) exactly as the home
    /// server holds them, `None` deleting one, and reload the schema. Returns whether the schema
    /// loaded: the feed can deliver a type before a taxonomy it names, and then the old schema
    /// stays until a later call completes the set.
    pub fn apply_remote_files(&self, files: &[(String, Option<String>)]) -> Result<bool> {
        let _w = self.write_lock();
        let mut store_changes = Vec::new();
        for (path, text) in files {
            if synced_file(path).is_none() {
                return Err(Error::BadRequest(format!("{path:?} is not a synced file")));
            }
            if in_store_layout(path) {
                store_changes.push(match text {
                    Some(t) => Change::write(path.clone(), t.as_bytes()),
                    None => Change::delete(path.clone()),
                });
                continue;
            }
            let full = self.vault.root().join(path);
            let io = |source| {
                Error::Store(store::Error::Io {
                    path: full.clone(),
                    source,
                })
            };
            match text {
                Some(t) => {
                    if let Some(dir) = full.parent() {
                        std::fs::create_dir_all(dir).map_err(io)?;
                    }
                    let tmp = full.with_extension("sync-tmp");
                    std::fs::write(&tmp, t).map_err(io)?;
                    std::fs::rename(&tmp, &full).map_err(io)?;
                }
                None => match std::fs::remove_file(&full) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(e)),
                    _ => {}
                },
            }
            self.state
                .journal_files(&[(path.clone(), text.is_some())])?;
        }
        let mut inner = self.lock();
        if !store_changes.is_empty() {
            let generation = self.commit(store_changes)?;
            inner.index.advance(generation)?;
        }
        match Schema::load(&self.vault) {
            Ok(schema) => {
                inner.index.set_schema(&schema)?;
                inner.schema = schema;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

/// The validation report the index keeps for a note: which items its type does not describe. A
/// note the local schema cannot validate is indexed all the same, with an empty report: the
/// server accepted it, and the schema that describes it may still be on its way.
fn report(schema: &Schema, w: &Writer, type_name: &str, doc: &schema::Doc) -> NoteReport {
    let lookup = crate::Lookup {
        writer: w,
        pending: Default::default(),
        deleted: Default::default(),
    };
    schema
        .validate_note(type_name, doc, &lookup)
        .unwrap_or_default()
}
