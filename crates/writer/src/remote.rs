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
//! - Git sync, between instances, writes another vault's text of a note the same way under an
//!   author of its own: [`Writer::create_remote`] and [`Writer::replace_remote`], which check the
//!   note is absent or at a base version so a user's edit is not lost (git-sync D7, D28).

use schema::{NoteReport, Schema};
use state::Author;
use store::Change;

use crate::{
    AGENT_THREAD_TYPE, Error, Event, Inner, Input, Result, Saved, Writer, feed::synced_file, first,
    note_path,
};

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

/// What a write from another vault expects of the note it writes over.
#[derive(Clone, Copy)]
enum Expect<'a> {
    /// Anything: the home server's copy is the master (D19).
    Any,
    /// No note under the id, live, trashed or in the history.
    Absent,
    /// A live note at this version (D12).
    At(&'a str),
}

/// Where a note stands in this vault: [`Writer::note_state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteState {
    /// In the vault, with its type and current version.
    Live { type_name: String, version: String },
    /// In the trash.
    Trashed,
    /// Neither: never created here, or purged from the trash.
    Absent,
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

    /// Create note `id` of `type_name` from `wcl`, another vault's text of it, recorded as
    /// `author`: git sync imports a note under the id it has everywhere (git-sync D7, D28). The
    /// text is written as it is, canonical, with its stamps; no `before_*` or `after_*` handler
    /// runs and no activity is recorded, as for [`Writer::apply_remote`]. `id` must be a ULID no
    /// note here has, live, in the trash or in the history, else [`Error::IdTaken`].
    pub fn create_remote(
        &self,
        id: &str,
        type_name: &str,
        wcl: &str,
        author: &Author,
    ) -> Result<Saved> {
        Self::check_memory_author(type_name, author)?;
        let _w = self.write_lock();
        let mut inner = self.lock();
        let wcl = self.remote_text(&inner, id, type_name, wcl)?;
        self.put_remote(&mut inner, id, type_name, &wcl, author, Expect::Absent)
    }

    /// Replace live note `id` with `wcl`, another vault's text of it, as type `type_name`,
    /// recorded as `author`, as [`Writer::create_remote`] writes. The note must be at version
    /// `base`, else [`Error::Conflict`], so a user's edit since `base` is not lost. Text the note
    /// already holds records nothing and returns the current version.
    pub fn replace_remote(
        &self,
        id: &str,
        type_name: &str,
        wcl: &str,
        base: &str,
        author: &Author,
    ) -> Result<Saved> {
        Self::check_memory_author(type_name, author)?;
        if let Some(old_type) = self.locate(id)? {
            Self::check_memory_author(&old_type, author)?;
        }
        let _w = self.write_lock();
        let mut inner = self.lock();
        if self.locate(id)?.as_deref() == Some(AGENT_THREAD_TYPE) {
            return Err(Error::ServerOwnedThread);
        }
        let wcl = self.remote_text(&inner, id, type_name, wcl)?;
        self.put_remote(&mut inner, id, type_name, &wcl, author, Expect::At(base))
    }

    /// Whether note `id` is live, in the trash, or neither: never created here, or purged. Git
    /// sync tells a note the user deleted from one it has yet to import this way (git-sync D17).
    pub fn note_state(&self, id: &str) -> Result<NoteState> {
        let _inner = self.lock();
        if let Some(type_name) = self.locate(id)? {
            let text = self.vault.read_text(&note_path(&type_name, id))?;
            let version = self.head(id, &type_name, &text)?;
            return Ok(NoteState::Live { type_name, version });
        }
        if self.state.trash_entry(id)?.is_some() {
            return Ok(NoteState::Trashed);
        }
        Ok(NoteState::Absent)
    }

    /// `wcl` in canonical form, for a type this vault has.
    fn remote_text(&self, inner: &Inner, id: &str, type_name: &str, wcl: &str) -> Result<String> {
        if type_name == AGENT_THREAD_TYPE {
            return Err(Error::ServerOwnedThread);
        }
        if !schema::is_ulid(id) || !schema::is_name(type_name) {
            return Err(Error::BadRequest(format!("{type_name}/{id} is not a note")));
        }
        if inner.schema.type_def(type_name).is_none() {
            return Err(Error::UnknownType(type_name.to_owned()));
        }
        Ok(store::canonical(wcl, &note_path(type_name, id))?)
    }

    /// Write note `id` as `wcl`, byte for byte, recorded as `author`, once the note is as
    /// `expect` says, and index it. No handler runs. The caller holds the write lock.
    fn put_remote(
        &self,
        inner: &mut Inner,
        id: &str,
        type_name: &str,
        wcl: &str,
        author: &Author,
        expect: Expect<'_>,
    ) -> Result<Saved> {
        if !schema::is_ulid(id) || !schema::is_name(type_name) {
            return Err(Error::BadRequest(format!("{type_name}/{id} is not a note")));
        }
        let path = note_path(type_name, id);
        let doc = Input::Wcl(wcl.to_owned()).parse(&path)?;
        let old_type = self.locate(id)?;
        match (expect, old_type.as_deref()) {
            (Expect::Any, _) => {}
            (Expect::Absent, None) if self.state.history(id)?.is_empty() => {}
            (Expect::Absent, _) => return Err(Error::IdTaken(id.to_owned())),
            (Expect::At(_), None) => return Err(Error::NotFound(id.to_owned())),
            (Expect::At(base), Some(old)) => {
                let text = self.vault.read_text(&note_path(old, id))?;
                self.check_base(id, old, &text, Some(base))?;
            }
        }
        // Only the authenticated home-server feed is authoritative. Peer imports
        // cannot introduce provider capabilities, even when their author is trusted.
        if !matches!(expect, Expect::Any) {
            self.check_remote_calendar(id, type_name, &doc)?;
        }
        if old_type.as_deref() == Some(type_name) && self.vault.read_text(&path)? == wcl {
            return Ok(Saved {
                id: id.to_owned(),
                type_name: type_name.to_owned(),
                event: Event::Update,
                version: self.head(id, type_name, wcl)?,
                wcl: wcl.to_owned(),
                previous: None,
                restored: false,
                author: None,
                also: Vec::new(),
            });
        }
        let mut files = Vec::new();
        if let Some(old) = old_type.as_deref().filter(|t| *t != type_name) {
            files.push(Change::delete(note_path(old, id)));
        }
        files.push(Change::write(path, wcl.as_bytes()));
        let report = report(&inner.schema, self, type_name, &doc);
        let generation = self.commit(files)?;
        let version = self.state.record_version(id, type_name, author, wcl)?;
        self.update_note_index(
            inner,
            &[index::Update::Put {
                id: id.to_owned(),
                type_name: type_name.to_owned(),
                doc,
                report,
            }],
        )?;
        inner.index.advance(generation)?;
        let saved = Saved {
            id: id.to_owned(),
            type_name: type_name.to_owned(),
            event: if old_type.is_some() {
                Event::Update
            } else {
                Event::Create
            },
            version: version.token,
            wcl: wcl.to_owned(),
            previous: None,
            restored: false,
            author: None,
            also: Vec::new(),
        };
        self.announce(std::slice::from_ref(&saved), &|_| None);
        Ok(saved)
    }

    /// Denial only: never run handlers or rewrite an untrusted incoming document.
    pub(crate) fn check_remote_calendar(
        &self,
        id: &str,
        type_name: &str,
        next: &schema::Doc,
    ) -> Result<()> {
        let old = match self.locate(id)? {
            Some(ty) => {
                let text = self.vault.read_text(&note_path(&ty, id))?;
                Some((ty.clone(), Input::Wcl(text).parse(&note_path(&ty, id))?))
            }
            None => None,
        };
        let calendar = |ty: &str| matches!(ty, "event" | "event_series");
        if !calendar(type_name) && !old.as_ref().is_some_and(|(ty, _)| calendar(ty)) {
            return Ok(());
        }
        let previous = old.as_ref().map(|(_, doc)| doc);
        let changed = [
            "google_ref", "etag", "attendees", "attendees_raw", "my_response", "status",
        ]
        .into_iter()
        .any(|name| next.field(name) != previous.and_then(|doc| doc.field(name)));
        let snapshots = |doc: &schema::Doc| {
            doc.blocks.iter()
                .filter(|b| b.kind == "dict" && b.label() == Some("snapshot"))
                .cloned().collect::<Vec<_>>()
        };
        let snapshot_changed = snapshots(next) != previous.map(snapshots).unwrap_or_default();
        let send = next.field("send_updates");
        let send_changed = send.is_some()
            && send != previous.and_then(|doc| doc.field("send_updates"));
        if changed || snapshot_changed || send_changed {
            return Err(Error::Rejected(
                "remote writes may not change protected calendar metadata or send_updates".into(),
            ));
        }
        Ok(())
    }

    /// Write a note exactly as the home server holds it, or move it to the trash, and index it.
    /// Returns the note's local version, or `None` when a delete found nothing to delete. A put
    /// of the text the note already holds records nothing and returns the current version.
    pub fn apply_remote(&self, change: &Remote) -> Result<Option<String>> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let author = Author::Token(REMOTE_AUTHOR.to_owned());
        match change {
            Remote::Put { id, type_name, wcl } => self
                .put_remote(&mut inner, id, type_name, wcl, &author, Expect::Any)
                .map(|s| Some(s.version)),
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
                self.update_note_index(inner, &[index::Update::Delete { id: id.clone() }])?;
                inner.index.advance(generation)?;
                self.announce(
                    &[Saved {
                        id: id.clone(),
                        type_name,
                        event: Event::Delete,
                        version: entry.version.clone(),
                        wcl: entry.content,
                        previous: None,
                        restored: false,
                        author: None,
                        also: Vec::new(),
                    }],
                    &|_| None,
                );
                Ok(Some(entry.version))
            }
        }
    }

    /// Write synced files (types, taxonomies, scripts, tasks, `devices/`, agents and the files
    /// of skills) exactly as the home server holds them, byte for byte, `None` deleting one, and
    /// reload the schema. Returns whether the schema loaded: the feed can deliver a type before a
    /// taxonomy it names, and then the old schema stays until a later call completes the set.
    pub fn apply_remote_files(&self, files: &[(String, Option<Vec<u8>>)]) -> Result<bool> {
        let _w = self.write_lock();
        let mut store_changes = Vec::new();
        let mut skills = std::collections::BTreeSet::new();
        // The files written outside a commit, announced once the schema is in place.
        let mut direct = Vec::new();
        for (path, text) in files {
            let Some((kind, name)) = synced_file(path) else {
                return Err(Error::BadRequest(format!("{path:?} is not a synced file")));
            };
            if kind == "skill" {
                skills.insert(name.to_owned());
            }
            if in_store_layout(path) {
                let exists = self.vault.root().join(path).is_file();
                store_changes.push(match text {
                    Some(t) => Change::write(path.clone(), t.clone()),
                    None if exists => Change::delete(path.clone()),
                    // Already gone here: nothing to delete.
                    None => continue,
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
            direct.push((path.clone(), text.is_some()));
        }
        let mut inner = self.lock();
        if !store_changes.is_empty() {
            let generation = self.commit(store_changes)?;
            inner.index.advance(generation)?;
        }
        for name in &skills {
            self.vault.prune_skill_folder(name)?;
        }
        let loaded = match Schema::load(&self.vault) {
            Ok(schema) => {
                inner.index.set_schema(&schema)?;
                inner.schema = schema;
                true
            }
            Err(_) => false,
        };
        self.announce_definitions(&direct)?;
        Ok(loaded)
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
