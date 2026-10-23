//! The single write path that every change goes through (D4, D12, D13, D20, D21).
//!
//! A write runs the steps of SPEC.md "The write path":
//!
//! 1. Parse the submitted note, WCL text or JSON (D32): [`Input`].
//! 2. Reject expressions and imports (D20).
//! 3. Validate against the type schema. A client cannot write a computed field (D3).
//! 4. Check the base version against the note's current one; a stale one is a [`Error::Conflict`]
//!    (D12).
//! 5. [`Hooks::before`]: the `before_*` handlers.
//! 6. [`Hooks::side_effects`]: reverse references and `on_delete` rules.
//! 7. Commit every file atomically (D4).
//! 8. Record a version of every changed note in `state.db`, or move a deleted note to the trash
//!    (D13).
//! 9. Update `index.db` and the tantivy index.
//! 10. [`Hooks::after`]: queue the `after_*` handlers.
//! 11. Return the canonical WCL and the new version of every changed note (D21).
//!
//! Step 6 is built in: [`refs`] works out the reverse sides of two-way and peer references and
//! applies `on_delete`. [`Hooks::side_effects`] can add more. [`NoHooks`] leaves steps 5 and 10
//! empty; the `script` crate's handlers fill them.
//!
//! Step 5 runs on every note the write changes, the side effects of step 6 included, so a note an
//! `unlink` edits runs its update handlers and a note a `cascade` deletes runs its delete handlers
//! (D5). Whatever the handlers change is expanded again, until nothing new needs a handler.
//!
//! Writes run one at a time, but the writer does not hold its data lock while handlers run, so a
//! handler can read and query the vault. A handler's own change to another note re-enters the
//! write path through [`Writer::stage_create`], [`Writer::stage_update`] and
//! [`Writer::stage_delete`]: it is checked like any write, runs its own `before_*` handlers, and
//! joins the write in progress, so it commits with it or not at all (D30).

mod asset_use;
mod assets;
mod batch;
mod builtin;
mod contacts;
mod daily;
mod defs;
mod devices;
mod feed;
mod fleeting;
pub mod google;
mod note;
mod notify;
pub mod outbox;
mod purge;
mod queryviews;
mod refs;
mod remote;
mod script_vault;
mod shipped;
mod terms;
mod time;
mod types;

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, MutexGuard};

use index::{Index, Update};
use schema::{Doc, Invalid, Issue, NoteLookup, NoteReport, Schema, TypeDef};
use state::{Author, Clock, State, SystemClock, TrashEntry, Version};
use store::{Change, CommitStep, Vault};

pub use asset_use::{ASSET_TRASH_DAYS, AssetSummary, AssetUsage, TrashedAsset, TrashedUse};
pub use assets::{Asset, MAX_FILENAME};
pub use batch::{Item, Op, Outcome, Report};
pub use builtin::{
    AGENT_THREAD_TYPE, BUILT_IN_KEYS, BUILT_IN_TAXONOMIES, BUILT_IN_TYPES, CALL_TYPE,
    CONVERSATION_TYPE, DEFAULT_AGENT_THREAD_TYPE, DEFAULT_CALL_TYPE, DEFAULT_CONVERSATION_TYPE,
    DEFAULT_EMAIL_THREAD_TYPE, DEFAULT_EMAIL_TYPE, DEFAULT_EVENT_TYPE,
    DEFAULT_GMAIL_LABEL_TAXONOMY, DEFAULT_MESSAGE_TYPE, DEFAULT_OUTBOX_TYPE, DEFAULT_PERSON_TYPE,
    EMAIL_THREAD_TYPE, EMAIL_TYPE, EVENT_TYPE, GMAIL_LABEL_TAXONOMY, MESSAGE_TYPE, OUTBOX_TYPE,
    PERSON_TYPE, TYPES_NEEDING_PERSON, is_built_in_type, matches_built_in,
};
pub use contacts::{CONTACTS_SOURCE, UPSERT_CONTACT, contact_link_key};
pub use daily::{DAILY_DATE_FIELD, DAILY_TYPE, DEFAULT_DAILY_TYPE, day_title, parse_day};
pub use defs::{ScriptProblem, TaxonomyCheck, agent_path};
pub use devices::{
    COMMAND_TTL_MS, DEFAULT_ANDROID_DEVICE_FILE, DEFAULT_DEVICE_SCRIPTS, MAX_COMMAND_PAYLOAD,
    command_json, default_device_file, default_device_script, device_file_path,
};
pub use feed::{SYNCED_FILES, synced_file};
pub use fleeting::{DEFAULT_FLEETING_TYPE, FLEETING_PROCESSED_FIELD, FLEETING_TYPE};
pub use index::{
    Archived, AssetHow, AssetUse, Extracted, Graph, GraphEdge, GraphNode, StoredText,
    extract_pdf_text,
};
pub use note::{
    ARCHIVED, CONFLICT_OF, Input, block_to_json, doc_to_json, is_archived, value_to_json, value_wcl,
};
pub use purge::Purged;
pub use refs::Referrer;
pub use remote::{REMOTE_AUTHOR, Remote};
pub use schema::NoteEdit;
pub use shipped::{
    SERVER_AUTHOR, SHIPPED_FILES, ShippedFile, UPGRADE_STEPS, UpgradeStep, Upgraded, Upkeep,
    content_hash, shipped_hash,
};
pub use types::{Flagged, Migration, PlannedNote, TypeChange, TypePlan};

/// What [`Writer::check_type`] found: the type file as the write would store it.
#[derive(Debug, Clone)]
pub struct TypeCheck {
    /// The canonical text of `types/<name>.wcl` after the write.
    pub wcl: String,
    /// Whether the type exists now, so the write changes it rather than creating it.
    pub exists: bool,
    /// What the write would ignore in the file, such as an attribute this version does not know.
    pub warnings: Vec<Issue>,
    /// For an existing type, the change as planned: the notes it edits and flags.
    pub plan: Option<TypePlan>,
}

/// Errors from the write path.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The submitted WCL does not parse.
    #[error("{file}: {message}")]
    Parse { file: String, message: String },
    /// The submitted WCL holds an expression, an import or another non-data item (D20).
    #[error(transparent)]
    DataOnly(store::DataOnlyError),
    /// The submitted JSON is not a note.
    #[error("{0}")]
    BadJson(String),
    /// The note does not validate against its type.
    #[error(transparent)]
    Invalid(Invalid),
    #[error("there is no type {0:?}")]
    UnknownType(String),
    #[error("note {0} does not exist")]
    NotFound(String),
    #[error("note {id} has no version {version:?}")]
    VersionNotFound { id: String, version: String },
    #[error("note {0} is not in the trash")]
    NotInTrash(String),
    /// A change to an existing note carried no base version (D12).
    #[error("a change to note {0} must carry the version it was based on")]
    BaseRequired(String),
    /// The base version is not the note's current version (D12).
    #[error("note {id} has changed: its current version is {current_version}")]
    Conflict {
        id: String,
        current_version: String,
        /// The note's current canonical WCL, for the client to compare.
        current: String,
    },
    /// A type that still has notes cannot be deleted.
    #[error("type {type_name:?} still has {} notes", .notes.len())]
    TypeHasNotes {
        type_name: String,
        notes: Vec<String>,
    },
    /// The type or its notes changed since the plan was made. Carries the plan as it is now.
    #[error("the plan is out of date: the type or its notes changed since it was made")]
    PlanStale(Box<TypePlan>),
    /// A delete is refused because notes refer to the note through `restrict` fields (D5).
    #[error(
        "note {id} cannot be deleted: {} refer to it with on_delete = \"restrict\"",
        describe_referrers(.referrers)
    )]
    Restricted {
        id: String,
        referrers: Vec<Referrer>,
    },
    /// A `before_*` handler refused the change (D9).
    #[error("{0}")]
    Rejected(String),
    /// A schema change's migration script failed on note `id` (D6).
    #[error("the migration failed on note {id}: {message}")]
    Migration { id: String, message: String },
    #[error("the vault's type definitions are invalid: {0}")]
    Schema(#[from] schema::SchemaError),
    /// A submitted type definition does not validate, alone or against the rest of the schema.
    #[error("{0}")]
    InvalidType(schema::SchemaError),
    /// Replacing this type could change its existing notes, which needs a planned schema change
    /// (D6).
    #[error("type {0:?} has notes, so changing it needs a planned schema change")]
    TypeInUse(String),
    /// A built-in type (`dailynote`, `fleeting`, `message`, `call`, `conversation`) can be changed
    /// but not deleted.
    #[error("{0:?} is built in: it can be customised but not deleted")]
    BuiltInType(String),
    /// The built-in agent (agents G6) cannot be deleted or disabled. Carries the whole message.
    #[error("{0}")]
    BuiltIn(String),
    /// A client chose an id for a new note or asset that is already in use, or that a note in
    /// the trash or the history once had.
    #[error("id {0} is already taken")]
    IdTaken(String),
    /// A request that is malformed in a way the other variants do not cover.
    #[error("{0}")]
    BadRequest(String),
    /// Something other than a note that the request names does not exist.
    #[error("{0}")]
    Missing(String),
    /// An upsert key matches more than one note.
    #[error("{} notes of type {type_name:?} share that {key:?}: {}", .ids.len(), .ids.join(", "))]
    KeyNotUnique {
        type_name: String,
        key: String,
        ids: Vec<String>,
    },
    /// A definition, term or asset cannot be removed while something uses it. `code` names what
    /// kind of thing it is for the API, and `by` lists the users.
    #[error("{message}: used by {}", .by.join(", "))]
    InUse {
        code: &'static str,
        message: String,
        by: Vec<String>,
    },
    /// An upgrade step cannot run yet, for the reason given: it is not recorded as applied, so it
    /// runs again on the next start.
    #[error("{0}")]
    StepDeferred(String),
    #[error(transparent)]
    Store(store::Error),
    #[error(transparent)]
    State(#[from] state::Error),
    #[error(transparent)]
    Index(#[from] index::Error),
}

impl Error {
    pub(crate) fn from_store(e: store::Error) -> Error {
        match e {
            store::Error::DataOnly(e) => Error::DataOnly(e),
            store::Error::Parse { file, message } => Error::Parse { file, message },
            other => Error::Store(other),
        }
    }
}

impl From<store::Error> for Error {
    fn from(e: store::Error) -> Error {
        Error::from_store(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a change does to a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    Create,
    Update,
    Delete,
}

/// One note change inside a write, after validation.
#[derive(Debug, Clone)]
pub struct NoteChange {
    pub id: String,
    pub type_name: String,
    pub event: Event,
    /// The note as it will be written. `None` for a delete.
    pub doc: Option<Doc>,
}

/// A note change that has been committed, recorded and indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub id: String,
    pub type_name: String,
    pub event: Event,
    /// The new version, or for a delete the version the note had when it went to the trash.
    pub version: String,
    /// The canonical WCL now on disk, or for a delete the WCL that went to the trash.
    pub wcl: String,
    /// The other notes the same write changed: reverse references and `on_delete` effects
    /// (D4, D5). Empty on those other notes themselves.
    pub also: Vec<Saved>,
}

/// How the vault runs a script, which decides the modules it may use (D30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScriptRole {
    /// A `before_*` handler.
    Before,
    /// An `after_*` handler.
    After,
    /// A scheduled task (D10).
    Task,
    /// A device event handler or device command, named by a device file (D5, D22).
    Device,
}

impl ScriptRole {
    /// The role as the API names it.
    pub fn as_str(self) -> &'static str {
        match self {
            ScriptRole::Before => "before_handler",
            ScriptRole::After => "after_handler",
            ScriptRole::Task => "task",
            ScriptRole::Device => "device",
        }
    }
}

/// The extension points of the write path. Every method defaults to doing nothing.
pub trait Hooks: Send + Sync {
    /// Step 5: run `before_*` handlers on one change. An error rejects the whole write with its
    /// message (D9).
    fn before(&self, change: &mut NoteChange, author: &Author) -> std::result::Result<(), String> {
        let _ = (change, author);
        Ok(())
    }

    /// Step 6: changes a write implies beyond what was submitted and beyond the built-in reverse
    /// references (D4) and `on_delete` rules (D5). They join the same commit.
    fn side_effects(&self, changes: &[NoteChange]) -> Result<Vec<NoteChange>> {
        let _ = changes;
        Ok(Vec::new())
    }

    /// Step 10: queue `after_*` handlers for the committed changes (D9). `schema` is the schema
    /// the write committed under.
    fn after(&self, saved: &[Saved], author: &Author, schema: &Schema) {
        let _ = (saved, author, schema);
    }

    /// Run `migration` over note `id` of `type_name` during a schema change (D6). `doc` is the
    /// note after the change's simple rules. `None` leaves it as it is.
    fn migrate(
        &self,
        type_name: &str,
        migration: &Migration,
        id: &str,
        doc: &Doc,
    ) -> std::result::Result<Option<Doc>, String> {
        let _ = (type_name, id, doc);
        Err(format!(
            "this vault runs no scripts, so migration {:?} cannot run",
            migration.script
        ))
    }

    /// Whether asset `id`, which is not in the vault folder, exists all the same: a device keeps
    /// only some assets and fetches the rest from the home server (D19). A note may name such an
    /// asset.
    fn asset_known(&self, id: &str) -> bool {
        let _ = id;
        false
    }

    /// Check that `source`, the text of script `name`, compiles when run as `role`, without
    /// running it. An error is the compiler's message.
    fn check_script(
        &self,
        role: ScriptRole,
        name: &str,
        source: &str,
    ) -> std::result::Result<(), String> {
        let _ = (role, name, source);
        Ok(())
    }
}

/// [`Hooks`] that do nothing.
pub struct NoHooks;

impl Hooks for NoHooks {}

/// A note as stored, with its current version.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: String,
    pub type_name: String,
    pub version: String,
    pub wcl: String,
    pub doc: Doc,
}

/// One row of a note listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: String,
    pub type_name: String,
    pub title: String,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub flagged: bool,
    /// Whether the note is archived.
    pub archived: bool,
    pub version: String,
}

/// The author recorded for a note found on disk with no version in `state.db`, such as a file
/// copied into the vault before the server started.
pub const ADOPTED_BY: &str = "vault";

/// The author recorded when opening the vault finds `state.db` behind the note files, as a crash
/// between steps 7 and 8 leaves it.
pub const RECOVERED_BY: &str = "recovery";

/// Bring `state.db` up to the note files. Opening the vault rolled forward any commit a crash
/// interrupted, but a crash after the commit point and before step 8 leaves `state.db` without the
/// versions of that commit. A note whose file differs from its current version gets a new version
/// holding the file's text, and a note whose file is gone moves to the trash. A file `state.db`
/// has never seen is left for [`Writer::head`] to adopt.
fn recover(vault: &Vault, state: &State) -> Result<()> {
    let author = Author::Session(RECOVERED_BY.to_owned());
    let mut on_disk = HashSet::new();
    for (type_name, id) in note_files(vault)? {
        if let Some(head) = state.head(&id)? {
            let text = vault.read_text(&note_path(&type_name, &id))?;
            if head.content != text || head.note_type != type_name {
                state.record_version(&id, &type_name, &author, &text)?;
            }
        }
        on_disk.insert(id);
    }
    for id in state.live_notes()? {
        if !on_disk.contains(&id) {
            state.trash_note(&id, &author)?;
        }
    }
    Ok(())
}

/// `(type, id)` of every `notes/<type>/<ULID>.wcl` file. Anything else under `notes/` is not a
/// note and is skipped.
fn note_files(vault: &Vault) -> Result<Vec<(String, String)>> {
    let read_dir = |dir: &std::path::Path| {
        std::fs::read_dir(dir).map_err(|source| {
            Error::Store(store::Error::Io {
                path: dir.to_owned(),
                source,
            })
        })
    };
    let mut out = Vec::new();
    for folder in read_dir(&vault.root().join("notes"))?.flatten() {
        let type_name = folder.file_name().to_string_lossy().into_owned();
        if !schema::is_name(&type_name) || !folder.path().is_dir() {
            continue;
        }
        for entry in read_dir(&folder.path())?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".wcl").filter(|id| schema::is_ulid(id))
                && entry.path().is_file()
            {
                out.push((type_name.clone(), id.to_owned()));
            }
        }
    }
    Ok(out)
}

/// The write path of one vault.
pub struct Writer {
    vault: Vault,
    state: State,
    clock: Arc<dyn Clock>,
    hooks: Box<dyn Hooks>,
    /// Held for the whole of every write, so writes run one at a time.
    writes: Mutex<()>,
    inner: Mutex<Inner>,
    /// The changes handlers made during step 5 of the write in progress, `None` outside step 5.
    staging: Mutex<Option<Vec<(NoteChange, Author)>>>,
}

/// Rounds of step 5 and step 6 one write may take before it is refused. Each round runs handlers
/// only on notes that have not yet run them, so only handlers that keep creating notes reach it.
const MAX_ROUNDS: usize = 32;

/// Called at each step of a commit; breaking stops the commit the way a crash would.
pub type CommitHook = Box<dyn FnMut(CommitStep) -> ControlFlow<()> + Send>;

/// What every write changes and every read sees. Writes hold it except while handlers run.
struct Inner {
    schema: Schema,
    index: Index,
    commit_hook: Option<CommitHook>,
}

impl Writer {
    /// Open the write path and load the schema. When the index is behind the vault's commits,
    /// bring `state.db` up to the vault files and rebuild the index from them. An index that is
    /// current is trusted as it is: every commit it holds also finished its `state.db` step, and
    /// files edited outside the server are not watched (D28), so `vault reindex` is how to pick
    /// them up. The change feed is brought up to the synced files on disk either way (D20).
    pub fn open(vault: Vault, state: State) -> Result<Writer> {
        let mut index = Index::open(&vault)?;
        if !index.is_current(&vault)? {
            recover(&vault, &state)?;
            index.rebuild(&vault)?;
        }
        feed::reconcile(&vault, &state)?;
        let schema = Schema::load(&vault)?;
        Ok(Writer {
            vault,
            state,
            clock: Arc::new(SystemClock),
            hooks: Box::new(NoHooks),
            writes: Mutex::new(()),
            inner: Mutex::new(Inner {
                schema,
                index,
                commit_hook: None,
            }),
            staging: Mutex::new(None),
        })
    }

    /// Read `created` and `updated` times from `clock`.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Writer {
        self.clock = clock;
        self
    }

    /// The time on the writer's clock, in milliseconds since the Unix epoch.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Run `hooks` at steps 5, 6 and 10.
    pub fn with_hooks(mut self, hooks: Box<dyn Hooks>) -> Writer {
        self.hooks = hooks;
        self
    }

    /// Pass every commit's steps to `hook`, which can stop a commit part way. For fault-injection
    /// tests.
    #[doc(hidden)]
    pub fn set_commit_hook(&self, hook: Option<CommitHook>) {
        self.lock().commit_hook = hook;
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// A copy of the current schema.
    pub fn schema(&self) -> Schema {
        self.lock().schema.clone()
    }

    /// Run `f` over the index and schema, with writes held off until it returns.
    pub fn read_index<T>(&self, f: impl FnOnce(&Index, &Schema) -> T) -> T {
        let inner = self.lock();
        f(&inner.index, &inner.schema)
    }

    /// Reload the types and taxonomies from the vault and refresh the index's terms.
    pub fn reload_schema(&self) -> Result<()> {
        let _w = self.write_lock();
        let schema = Schema::load(&self.vault)?;
        let mut inner = self.lock();
        inner.index.set_schema(&schema)?;
        inner.schema = schema;
        Ok(())
    }

    /// The canonical text of `types/<name>.wcl`.
    pub fn read_type(&self, name: &str) -> Result<String> {
        let inner = self.lock();
        if !inner_has_type(&inner, name) {
            return Err(Error::UnknownType(name.to_owned()));
        }
        Ok(self.vault.read_text(&type_path(name))?)
    }

    /// Create type `name`, or replace it while it has no notes, and reload the schema. Changing a
    /// type that has notes goes through a planned schema change instead (D6).
    pub fn put_type(&self, name: &str, source: &str) -> Result<String> {
        let mut texts = self.put_types(&[(name, source)])?;
        Ok(texts.remove(0))
    }

    /// Write several type files in one commit, checked as one schema, so types that name each
    /// other can arrive together. Returns each canonical text, in order.
    pub(crate) fn put_types(&self, sources: &[(&str, &str)]) -> Result<Vec<String>> {
        let _w = self.write_lock();
        let mut inner = self.lock();
        let (changes, texts, schema) = self.checked_types(&inner, sources)?;
        let generation = self.commit(changes)?;
        inner.index.set_schema(&schema)?;
        inner.index.advance(generation)?;
        inner.schema = schema;
        Ok(texts)
    }

    /// Everything [`Writer::put_types`] checks, without writing: the file changes, each canonical
    /// text, and the schema the write would install. The caller holds the write lock.
    fn checked_types(
        &self,
        inner: &Inner,
        sources: &[(&str, &str)],
    ) -> Result<(Vec<Change>, Vec<String>, Schema)> {
        let mut types = self.definitions("types")?;
        let mut changes = Vec::new();
        let mut texts = Vec::new();
        for &(name, source) in sources {
            let path = type_path(name);
            if !schema::is_name(name) {
                return Err(Error::InvalidType(schema::SchemaError(vec![Invalid {
                    file: path,
                    issues: vec![Issue::new("type", format!("{name:?} is not a type name"))],
                }])));
            }
            let text = store::canonical(source, &path)?;
            if inner_has_type(inner, name) {
                let has_notes = inner
                    .index
                    .connection()
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM notes WHERE type = ?1)",
                        [name],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(index::Error::from)?;
                if has_notes {
                    return Err(Error::TypeInUse(name.to_owned()));
                }
            }
            types.retain(|(n, _)| n != name);
            types.push((name.to_owned(), text.clone()));
            changes.push(Change::write(path, text.clone()));
            texts.push(text);
        }
        let taxonomies = self.definitions("taxonomies")?;
        let schema = Schema::from_sources(&pairs(&types), &pairs(&taxonomies))
            .map_err(Error::InvalidType)?;
        builtin::check_built_in_types(&schema)?;
        Ok((changes, texts, schema))
    }

    /// Check a write of type `name` from `source` without making it: a new type exactly as
    /// [`Writer::put_type`] would create it, and an existing one as a planned change with no
    /// renames, which is how a type that has notes changes (D6). The AI chat's
    /// `propose_type_change` uses it (ai D18).
    pub fn check_type(&self, name: &str, source: &str) -> Result<TypeCheck> {
        let exists = self.lock().schema.type_def(name).is_some();
        if exists {
            let change = TypeChange {
                wcl: source.to_owned(),
                ..TypeChange::default()
            };
            let plan = self.plan_type_change(name, &change)?;
            let warnings = TypeDef::parse(name, &plan.wcl)
                .map(|t| t.warnings)
                .unwrap_or_default();
            return Ok(TypeCheck {
                wcl: plan.wcl.clone(),
                exists: true,
                warnings,
                plan: Some(plan),
            });
        }
        let _w = self.write_lock();
        let inner = self.lock();
        let (_, mut texts, schema) = self.checked_types(&inner, &[(name, source)])?;
        let warnings = schema
            .type_def(name)
            .map(|t| t.warnings.clone())
            .unwrap_or_default();
        Ok(TypeCheck {
            wcl: texts.remove(0),
            exists: false,
            warnings,
            plan: None,
        })
    }

    /// The `(stem, text)` of every definition file in `dir`.
    fn definitions(&self, dir: &str) -> Result<Vec<(String, String)>> {
        let root = self.vault.root().join(dir);
        let entries = std::fs::read_dir(&root).map_err(|source| {
            Error::Store(store::Error::Io {
                path: root.clone(),
                source,
            })
        })?;
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = file_name.strip_suffix(".wcl") {
                let text = self.vault.read_text(&format!("{dir}/{file_name}"))?;
                out.push((stem.to_owned(), text));
            }
        }
        Ok(out)
    }

    /// Create a note of `type_name` with a new id.
    pub fn create(&self, type_name: &str, input: &Input, author: &Author) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let id = ulid::Ulid::new().to_string();
        let doc = input.parse(&note_path(type_name, &id))?;
        let change = self.prepare_put(&inner, &id, type_name, doc, None, author)?;
        self.apply(inner, vec![change], &|| Ok(()), author)
            .map(first)
    }

    /// Replace note `id`, which must be at version `base`.
    pub fn update(
        &self,
        id: &str,
        input: &Input,
        base: Option<&str>,
        author: &Author,
    ) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let (type_name, old) = self.existing(id)?;
        let doc = input.parse(&note_path(&type_name, id))?;
        let change = self.prepare_put(&inner, id, &type_name, doc, Some(&old), author)?;
        let check = || self.check_base(id, &type_name, &old, base);
        self.apply(inner, vec![change], &check, author).map(first)
    }

    /// Delete note `id`, which must be at version `base`, moving it to the trash.
    pub fn delete(&self, id: &str, base: Option<&str>, author: &Author) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let (type_name, old) = self.existing(id)?;
        self.check_base(id, &type_name, &old, base)?;
        let change = NoteChange {
            id: id.to_owned(),
            type_name,
            event: Event::Delete,
            doc: None,
        };
        self.apply(inner, vec![change], &|| Ok(()), author)
            .map(first)
    }

    /// Write an earlier version of note `id` back as a new version. The note must be at `base`.
    pub fn restore_version(
        &self,
        id: &str,
        version: &str,
        base: Option<&str>,
        author: &Author,
    ) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let (type_name, old) = self.existing(id)?;
        let past = self.version(id, version)?;
        let doc = Input::Wcl(past.content).parse(&note_path(&type_name, id))?;
        let change = self.prepare_put(&inner, id, &type_name, doc, Some(&old), author)?;
        let check = || self.check_base(id, &type_name, &old, base);
        self.apply(inner, vec![change], &check, author).map(first)
    }

    /// Bring note `id` back from the trash with its last content, as a new version.
    pub fn restore_from_trash(&self, id: &str, author: &Author) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        if let Some(type_name) = self.locate(id)? {
            let current = self.vault.read_text(&note_path(&type_name, id))?;
            let current_version = self.head(id, &type_name, &current)?;
            return Err(Error::Conflict {
                id: id.to_owned(),
                current_version,
                current,
            });
        }
        let entry = self
            .trash()?
            .into_iter()
            .find(|t| t.note_id == id)
            .ok_or_else(|| Error::NotInTrash(id.to_owned()))?;
        let doc = Input::Wcl(entry.content.clone()).parse(&note_path(&entry.note_type, id))?;
        let created = created_of(&doc);
        // A note archived when it was deleted comes back archived.
        let archived = note::is_archived(&doc);
        // Its computed values are its own, written by handlers before it was deleted, so they
        // come back with it.
        let own = doc.clone();
        let mut change = self.prepare_put_as(
            &inner,
            id,
            &entry.note_type,
            doc,
            None,
            author,
            Some(archived),
            Some(&own),
        )?;
        // The note existed before; it keeps the time it was first created.
        if let (Some(created), Some(doc)) = (created, change.doc.as_mut()) {
            set_field(doc, "created", created);
        }
        change.event = Event::Create;
        self.apply(inner, vec![change], &|| Ok(()), author)
            .map(first)
    }

    /// Archive note `id`, or take it out of the archive, as a new version. The note must be at
    /// `base`. The change is an update like any other: it is recorded in the history, stamps
    /// `updated` and runs the type's update handlers. A note already in the state asked for is
    /// left as it is, and its current version returned.
    pub fn set_archived(
        &self,
        id: &str,
        archived: bool,
        base: Option<&str>,
        author: &Author,
    ) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let (type_name, old) = self.existing(id)?;
        self.check_base(id, &type_name, &old, base)?;
        let doc = Input::Wcl(old.clone()).parse(&note_path(&type_name, id))?;
        if note::is_archived(&doc) == archived {
            let version = self.head(id, &type_name, &old)?;
            return Ok(Saved {
                id: id.to_owned(),
                type_name,
                event: Event::Update,
                version,
                wcl: old,
                also: Vec::new(),
            });
        }
        let change = self.prepare_put_as(
            &inner,
            id,
            &type_name,
            doc,
            Some(&old),
            author,
            Some(archived),
            None,
        )?;
        let check = || self.check_base(id, &type_name, &old, base);
        self.apply(inner, vec![change], &check, author).map(first)
    }

    /// [`Writer::set_archived`] inside the write in progress, for a `before_*` handler. See
    /// [`Writer::stage_create`].
    pub fn stage_set_archived(
        &self,
        id: &str,
        archived: bool,
        base: Option<&str>,
        author: &Author,
    ) -> Result<NoteChange> {
        self.check_staging()?;
        let (type_name, old) = self.existing(id)?;
        self.check_base(id, &type_name, &old, base)?;
        let doc = Input::Wcl(old.clone()).parse(&note_path(&type_name, id))?;
        let change = self.prepare_put_as(
            &self.lock(),
            id,
            &type_name,
            doc,
            Some(&old),
            author,
            Some(archived),
            None,
        )?;
        self.stage(change, author)
    }

    /// Create a note of `type_name` inside the write in progress, for a `before_*` handler
    /// (D30). The change runs its own `before_*` handlers and commits with the write. Returns the
    /// change as it will be written.
    pub fn stage_create(
        &self,
        type_name: &str,
        input: &Input,
        author: &Author,
    ) -> Result<NoteChange> {
        self.check_staging()?;
        let id = ulid::Ulid::new().to_string();
        let doc = input.parse(&note_path(type_name, &id))?;
        let change = self.prepare_put(&self.lock(), &id, type_name, doc, None, author)?;
        self.stage(change, author)
    }

    /// Replace note `id`, which must be at version `base`, inside the write in progress. See
    /// [`Writer::stage_create`].
    pub fn stage_update(
        &self,
        id: &str,
        input: &Input,
        base: Option<&str>,
        author: &Author,
    ) -> Result<NoteChange> {
        self.check_staging()?;
        let (type_name, old) = self.existing(id)?;
        let doc = input.parse(&note_path(&type_name, id))?;
        let change = self.prepare_put(&self.lock(), id, &type_name, doc, Some(&old), author)?;
        self.check_base(id, &type_name, &old, base)?;
        self.stage(change, author)
    }

    /// Delete note `id`, which must be at version `base`, inside the write in progress. See
    /// [`Writer::stage_create`].
    pub fn stage_delete(
        &self,
        id: &str,
        base: Option<&str>,
        author: &Author,
    ) -> Result<NoteChange> {
        self.check_staging()?;
        let (type_name, old) = self.existing(id)?;
        self.check_base(id, &type_name, &old, base)?;
        let change = NoteChange {
            id: id.to_owned(),
            type_name,
            event: Event::Delete,
            doc: None,
        };
        self.stage(change, author)
    }

    fn check_staging(&self) -> Result<()> {
        if self.staging_slot().is_none() {
            return Err(Error::Rejected(
                "a note change can join a write only while its before_* handlers run".into(),
            ));
        }
        Ok(())
    }

    /// Run a staged change's `before_*` handlers and add it to the write in progress.
    fn stage(&self, mut change: NoteChange, author: &Author) -> Result<NoteChange> {
        self.before(&mut change, author)?;
        match self.staging_slot().as_mut() {
            Some(staged) => staged.push((change.clone(), author.clone())),
            None => {
                return Err(Error::Rejected(
                    "the write this change was to join has ended".into(),
                ));
            }
        }
        Ok(change)
    }

    fn staging_slot(&self) -> MutexGuard<'_, Option<Vec<(NoteChange, Author)>>> {
        self.staging.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Read note `id`.
    pub fn read(&self, id: &str) -> Result<Note> {
        let _inner = self.lock();
        let (type_name, wcl) = self.existing(id)?;
        let version = self.head(id, &type_name, &wcl)?;
        let doc = Input::Wcl(wcl.clone()).parse(&note_path(&type_name, id))?;
        Ok(Note {
            id: id.to_owned(),
            type_name,
            version,
            wcl,
            doc,
        })
    }

    /// The indexed title of note `id`, if the index holds the note.
    pub fn title(&self, id: &str) -> Result<Option<String>> {
        let inner = self.lock();
        let conn = inner.index.connection();
        let mut stmt = conn
            .prepare("SELECT title FROM notes WHERE id = ?1")
            .map_err(index::Error::from)?;
        let mut rows = stmt
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(index::Error::from)?;
        Ok(rows.next().transpose().map_err(index::Error::from)?)
    }

    /// The link graph around note `id`: the notes it links to and that link to it, at most
    /// `limit` of them, most recently updated first, and every link among them.
    pub fn graph(&self, id: &str, limit: usize) -> Result<index::Graph> {
        let inner = self.lock();
        inner
            .index
            .graph(id, limit)?
            .ok_or_else(|| Error::NotFound(id.to_owned()))
    }

    /// The type and indexed title of note `id`, if the index holds the note. A note in the trash
    /// is not indexed.
    pub fn type_and_title(&self, id: &str) -> Result<Option<(String, String)>> {
        let inner = self.lock();
        let conn = inner.index.connection();
        let mut stmt = conn
            .prepare("SELECT type, title FROM notes WHERE id = ?1")
            .map_err(index::Error::from)?;
        let mut rows = stmt
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(index::Error::from)?;
        Ok(rows.next().transpose().map_err(index::Error::from)?)
    }

    /// Every indexed note that is not archived, or every such note of one type, oldest first.
    pub fn list(&self, type_name: Option<&str>) -> Result<Vec<Summary>> {
        self.list_with(type_name, Archived::Exclude)
    }

    /// [`Writer::list`], with archived notes left out, kept or the only ones listed.
    pub fn list_with(&self, type_name: Option<&str>, archived: Archived) -> Result<Vec<Summary>> {
        let inner = self.lock();
        let conn = inner.index.connection();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, type, title, created, updated, flagged, archived FROM notes
                 WHERE (?1 IS NULL OR type = ?1) AND {} ORDER BY id",
                archived.sql("archived")
            ))
            .map_err(index::Error::from)?;
        let rows = stmt
            .query_map([type_name], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })
            .map_err(index::Error::from)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, type_name, title, created, updated, flagged, archived) =
                row.map_err(index::Error::from)?;
            let wcl = self.vault.read_text(&note_path(&type_name, &id))?;
            let version = self.head(&id, &type_name, &wcl)?;
            out.push(Summary {
                id,
                type_name,
                title,
                created,
                updated,
                flagged,
                archived,
                version,
            });
        }
        Ok(out)
    }

    /// Every recorded version of note `id`, oldest first, live or trashed.
    pub fn history(&self, id: &str) -> Result<Vec<Version>> {
        let _inner = self.lock();
        if let Some(type_name) = self.locate(id)? {
            let wcl = self.vault.read_text(&note_path(&type_name, id))?;
            self.head(id, &type_name, &wcl)?;
        }
        let versions = self.state.history(id)?;
        if versions.is_empty() {
            return Err(Error::NotFound(id.to_owned()));
        }
        Ok(versions)
    }

    /// The current version token of a live note, or `None` when it is not live.
    pub fn current_version(&self, id: &str) -> Result<Option<String>> {
        Ok(self.state.current_version(id)?)
    }

    /// One recorded version of note `id`.
    pub fn version(&self, id: &str, version: &str) -> Result<Version> {
        self.state
            .version(id, version)?
            .ok_or_else(|| Error::VersionNotFound {
                id: id.to_owned(),
                version: version.to_owned(),
            })
    }

    /// Every note in the trash, most recently deleted first.
    pub fn trash(&self) -> Result<Vec<TrashEntry>> {
        Ok(self.state.list_trash()?)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write_lock(&self) -> MutexGuard<'_, ()> {
        self.writes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The type folder holding note `id`, from the files on disk.
    fn locate(&self, id: &str) -> Result<Option<String>> {
        if !schema::is_ulid(id) {
            return Ok(None);
        }
        let notes = self.vault.root().join("notes");
        let entries = std::fs::read_dir(&notes).map_err(|source| {
            Error::Store(store::Error::Io {
                path: notes.clone(),
                source,
            })
        })?;
        for entry in entries.flatten() {
            let type_name = entry.file_name().to_string_lossy().into_owned();
            if schema::is_name(&type_name) && entry.path().join(format!("{id}.wcl")).is_file() {
                return Ok(Some(type_name));
            }
        }
        Ok(None)
    }

    /// The type and current text of a live note.
    fn existing(&self, id: &str) -> Result<(String, String)> {
        let type_name = self
            .locate(id)?
            .ok_or_else(|| Error::NotFound(id.to_owned()))?;
        let text = self.vault.read_text(&note_path(&type_name, id))?;
        Ok((type_name, text))
    }

    /// The current version of a live note. A note on disk that `state.db` has never seen is
    /// adopted: its text is recorded as its first version.
    fn head(&self, id: &str, type_name: &str, text: &str) -> Result<String> {
        if let Some(v) = self.state.current_version(id)? {
            return Ok(v);
        }
        let author = Author::Session(ADOPTED_BY.to_owned());
        Ok(self
            .state
            .record_version(id, type_name, &author, text)?
            .token)
    }

    /// Step 4.
    fn check_base(&self, id: &str, type_name: &str, text: &str, base: Option<&str>) -> Result<()> {
        let base = base.ok_or_else(|| Error::BaseRequired(id.to_owned()))?;
        let current_version = self.head(id, type_name, text)?;
        if base == current_version {
            Ok(())
        } else {
            Err(Error::Conflict {
                id: id.to_owned(),
                current_version,
                current: text.to_owned(),
            })
        }
    }

    /// Steps 3 and the canonical layout: arrange the submitted doc, stamp `created` and
    /// `updated`, and validate it. `old` is the note's current text for an update. The note keeps
    /// the archived state it has: a client cannot archive a note by writing it.
    fn prepare_put(
        &self,
        inner: &Inner,
        id: &str,
        type_name: &str,
        doc: Doc,
        old: Option<&str>,
        author: &Author,
    ) -> Result<NoteChange> {
        self.prepare_put_as(inner, id, type_name, doc, old, author, None, None)
    }

    /// [`Writer::prepare_put`] that sets the note's archived state to `archived`, or keeps the
    /// state the note has when `None`. A new note is not archived. `computed_from` is the note
    /// whose computed values the write may keep, when not `old`: the note's own last content, for
    /// a note restored from the trash.
    #[allow(clippy::too_many_arguments)]
    fn prepare_put_as(
        &self,
        inner: &Inner,
        id: &str,
        type_name: &str,
        mut doc: Doc,
        old: Option<&str>,
        author: &Author,
        archived: Option<bool>,
        computed_from: Option<&Doc>,
    ) -> Result<NoteChange> {
        let ty = inner
            .schema
            .type_def(type_name)
            .ok_or_else(|| Error::UnknownType(type_name.to_owned()))?;
        let path = note_path(type_name, id);
        let now = time::utc(self.clock.now_ms());
        let old_doc = old
            .map(|t| Input::Wcl(t.to_owned()).parse(&path))
            .transpose()?;
        let created = old_doc
            .as_ref()
            .and_then(created_of)
            .unwrap_or_else(|| now.clone());
        if !matches!(author, Author::Handler(_) | Author::Task(_)) {
            let issues = computed_writes(ty, &doc, computed_from.or(old_doc.as_ref()));
            if !issues.is_empty() {
                return Err(Error::Invalid(Invalid { file: path, issues }));
            }
        }
        let outbox = type_name == OUTBOX_TYPE;
        if outbox {
            self.fill_outbox(&mut doc, old.is_none(), author);
        }
        // A new note gets the default of every field it omits; an update does not.
        if old.is_none() {
            for f in &ty.fields {
                if let Some(value) = f.default.as_ref().filter(|_| doc.field(&f.name).is_none()) {
                    doc.fields.push((f.name.clone(), value.clone()));
                }
            }
        }
        if outbox {
            self.check_outbox(ty, &path, &doc, old_doc.as_ref(), author)?;
        }
        let archived = archived.unwrap_or_else(|| old_doc.as_ref().is_some_and(note::is_archived));
        note::set_archived(&mut doc, archived);
        let doc = note::arrange(ty, doc, &created, &now);
        Ok(NoteChange {
            id: id.to_owned(),
            type_name: type_name.to_owned(),
            event: if old.is_some() {
                Event::Update
            } else {
                Event::Create
            },
            doc: Some(doc),
        })
    }

    /// Validate (step 3), run `check_base` (step 4), then steps 5 to 11 for one write. The caller
    /// holds the write lock; `inner` is released while handlers run.
    fn apply(
        &self,
        inner: MutexGuard<'_, Inner>,
        changes: Vec<NoteChange>,
        check_base: &dyn Fn() -> Result<()>,
        author: &Author,
    ) -> Result<Vec<Saved>> {
        self.apply_with(inner, changes, check_base, author, Vec::new(), false)
    }

    /// [`Writer::apply`], committing `files` in the same commit as the notes. A `dry_run` stops
    /// before the commit and returns what the write would save: each put with an empty version
    /// and the text it would write, each delete with its current version and text. A dry run
    /// runs `before_*` handlers, since they change what would be saved, and queues no `after_*`
    /// handlers.
    fn apply_with(
        &self,
        inner: MutexGuard<'_, Inner>,
        changes: Vec<NoteChange>,
        check_base: &dyn Fn() -> Result<()>,
        author: &Author,
        extra_files: Vec<Change>,
        dry_run: bool,
    ) -> Result<Vec<Saved>> {
        // Validate what was submitted before any handler sees it. A term new to an open
        // taxonomy is checked as the taxonomy will be once the write adds it.
        let mut changes = changes;
        match terms::Growth::changes(&inner.schema, &mut changes)?.finish(&inner.schema)? {
            Some((grown, _)) => self.render(&grown, &changes)?,
            None => self.render(&inner.schema, &changes)?,
        };
        check_base()?;
        drop(inner);

        // Steps 5 and 6, until every note the write changes has run its `before_*` handlers.
        let mut handled = HashSet::new();
        let mut authors = HashMap::new();
        let mut changes = self.run_before(changes, &mut handled, &mut authors, author)?;
        let mut inner = self.lock();
        let mut rounds = 0;
        let changes = loop {
            let expanded = refs::expand(self, &inner, changes)?;
            if expanded
                .iter()
                .all(|c| handled.contains(&(c.id.clone(), c.event)))
            {
                break expanded;
            }
            rounds += 1;
            if rounds > MAX_ROUNDS {
                return Err(Error::Rejected(format!(
                    "the write's handlers kept changing more notes after {MAX_ROUNDS} rounds"
                )));
            }
            drop(inner);
            changes = self.run_before(expanded, &mut handled, &mut authors, author)?;
            inner = self.lock();
        };
        let mut changes = changes;
        let extra = self.hooks.side_effects(&changes)?;
        changes.extend(extra);

        // Handlers may have changed the notes, so what is written is rendered and checked again,
        // and the terms the notes add to open taxonomies are worked out from what they hold now.
        let grown = terms::Growth::changes(&inner.schema, &mut changes)?.finish(&inner.schema)?;
        let rendered = self.render(
            grown.as_ref().map_or(&inner.schema, |(schema, _)| schema),
            &changes,
        )?;

        if dry_run {
            return rendered
                .into_iter()
                .map(|r| {
                    let NoteChange {
                        id,
                        type_name,
                        event,
                        ..
                    } = r.change;
                    let (version, wcl) = match r.text {
                        Some(text) => (String::new(), text),
                        None => {
                            let text = self.vault.read_text(&note_path(&type_name, &id))?;
                            (self.head(&id, &type_name, &text)?, text)
                        }
                    };
                    Ok(Saved {
                        id,
                        type_name,
                        event,
                        version,
                        wcl,
                        also: Vec::new(),
                    })
                })
                .collect();
        }

        let (grown, mut files) = match grown {
            Some((schema, taxonomies)) => (Some(schema), [extra_files, taxonomies].concat()),
            None => (None, extra_files),
        };
        // A change of archived state always gets a version of its own, so the history shows it
        // even when the same session saved the note a moment before.
        let mut flips = HashSet::new();
        for r in &rendered {
            if let (Event::Update, Some((doc, _))) = (r.change.event, &r.doc) {
                let path = note_path(&r.change.type_name, &r.change.id);
                let was = self
                    .vault
                    .read_text(&path)
                    .is_ok_and(|t| t.lines().any(|l| l == "archived = true"));
                if was != note::is_archived(doc) {
                    flips.insert(r.change.id.clone());
                }
            }
        }
        files.reserve(rendered.len());
        for r in &rendered {
            let path = note_path(&r.change.type_name, &r.change.id);
            files.push(match &r.text {
                Some(text) => Change::write(path, text.as_bytes()),
                None => Change::delete(path),
            });
        }
        let generation = match inner.commit_hook.as_mut() {
            Some(hook) => self.commit_with_hook(files, &mut **hook)?,
            None => self.commit(files)?,
        };

        let mut saved = Vec::with_capacity(rendered.len());
        let mut updates = Vec::with_capacity(rendered.len());
        let mut people = Vec::new();
        for r in rendered {
            let NoteChange {
                id,
                type_name,
                event,
                ..
            } = r.change;
            let by = authors.get(&id).unwrap_or(author);
            match (r.text, r.doc) {
                (Some(text), Some((doc, report))) => {
                    if type_name == PERSON_TYPE {
                        let before = match event {
                            Event::Update => self
                                .state
                                .head(&id)?
                                .and_then(|v| {
                                    Input::Wcl(v.content)
                                        .parse(&note_path(&type_name, &id))
                                        .ok()
                                })
                                .map(|d| doc_to_json(&d)),
                            _ => None,
                        };
                        people.push(contacts::PersonChange {
                            id: id.clone(),
                            author: by.clone(),
                            before,
                            after: doc_to_json(&doc),
                        });
                    }
                    let v = if flips.contains(&id) {
                        self.state
                            .record_distinct_version(&id, &type_name, by, &text)?
                    } else {
                        self.state.record_version(&id, &type_name, by, &text)?
                    };
                    updates.push(Update::Put {
                        id: id.clone(),
                        type_name: type_name.clone(),
                        doc,
                        report,
                    });
                    saved.push(Saved {
                        id,
                        type_name,
                        event,
                        version: v.token,
                        wcl: text,
                        also: Vec::new(),
                    });
                }
                _ => {
                    let entry = self.state.trash_note(&id, by)?;
                    updates.push(Update::Delete { id: id.clone() });
                    saved.push(Saved {
                        id,
                        type_name,
                        event,
                        version: entry.version,
                        wcl: entry.content,
                        also: Vec::new(),
                    });
                }
            }
        }
        let inner = &mut *inner;
        if let Some(schema) = grown {
            inner.index.set_schema(&schema)?;
            inner.schema = schema;
        }
        inner.index.update(&inner.schema, &updates)?;
        inner.index.advance(generation)?;
        self.announce(&saved, &|id| match authors.get(id).unwrap_or(author) {
            Author::Token(name) => Some(name.clone()),
            _ => None,
        });
        self.hooks.after(&saved, author, &inner.schema);
        self.people_to_contacts(&people);
        Ok(saved)
    }

    /// Tell open clients about committed changes (D4). The caller holds the write lock, so the
    /// events go out in commit order.
    /// `token` names the API token that made the change to a note, when a token did.
    pub(crate) fn announce(&self, saved: &[Saved], token: &dyn Fn(&str) -> Option<String>) {
        for s in saved {
            self.state.events().publish(state::LiveEvent::NoteChanged {
                id: s.id.clone(),
                type_name: s.type_name.clone(),
                version: s.version.clone(),
                deleted: s.event == Event::Delete,
                token: token(&s.id),
            });
        }
    }

    /// Step 5 for every change not yet in `handled`, with the changes its handlers staged
    /// merged in. A staged change to a note the write already changes replaces that change, and
    /// `authors` records the handler that staged it.
    fn run_before(
        &self,
        mut changes: Vec<NoteChange>,
        handled: &mut HashSet<(String, Event)>,
        authors: &mut HashMap<String, Author>,
        author: &Author,
    ) -> Result<Vec<NoteChange>> {
        *self.staging_slot() = Some(Vec::new());
        let ran = (|| -> Result<()> {
            for change in &mut changes {
                if handled.insert((change.id.clone(), change.event)) {
                    self.before(change, author)?;
                }
            }
            Ok(())
        })();
        let staged = self.staging_slot().take().unwrap_or_default();
        ran?;
        for (change, by) in staged {
            handled.insert((change.id.clone(), change.event));
            authors.insert(change.id.clone(), by);
            match changes.iter_mut().find(|c| c.id == change.id) {
                Some(c) => *c = change,
                None => changes.push(change),
            }
        }
        Ok(changes)
    }

    /// Run the `before_*` handlers of one change. A note a handler changed keeps its `created`
    /// and `updated` times and its archived state, and is put back in canonical order.
    fn before(&self, change: &mut NoteChange, author: &Author) -> Result<()> {
        let archived = change.doc.as_ref().is_some_and(note::is_archived);
        let stamps = change.doc.as_ref().map(|d| {
            (
                created_of(d),
                d.field("updated")
                    .and_then(schema::Value::as_str)
                    .map(str::to_owned),
            )
        });
        let original = change.doc.clone();
        self.hooks.before(change, author).map_err(Error::Rejected)?;
        if change.doc == original {
            return Ok(());
        }
        let Some((Some(created), Some(updated))) = stamps else {
            return Ok(());
        };
        let Some(mut doc) = change.doc.take() else {
            return Ok(());
        };
        note::set_archived(&mut doc, archived);
        let inner = self.lock();
        let ty = inner
            .schema
            .type_def(&change.type_name)
            .ok_or_else(|| Error::UnknownType(change.type_name.clone()))?;
        change.doc = Some(note::arrange(ty, doc, &created, &updated));
        Ok(())
    }

    /// The canonical text of every put and the validation of the note it holds. A delete renders
    /// to nothing, after making sure `state.db` holds the content it will trash.
    fn render(&self, schema: &Schema, changes: &[NoteChange]) -> Result<Vec<Rendered>> {
        let mut pending = HashMap::new();
        let mut deleted = HashSet::new();
        for c in changes {
            match c.event {
                Event::Delete => {
                    deleted.insert(c.id.clone());
                }
                _ => {
                    pending.insert(c.id.clone(), c.type_name.clone());
                }
            }
        }
        let lookup = Lookup {
            writer: self,
            pending,
            deleted,
        };
        let mut out = Vec::with_capacity(changes.len());
        for c in changes {
            let path = note_path(&c.type_name, &c.id);
            let Some(doc) = &c.doc else {
                let text = self.vault.read_text(&path)?;
                self.head(&c.id, &c.type_name, &text)?;
                out.push(Rendered {
                    change: c.clone(),
                    text: None,
                    doc: None,
                });
                continue;
            };
            // A canvas node of a kind cards replaced is written as its card, never as it came.
            let mut doc = doc.clone();
            schema::migrate::canvases(&mut doc);
            let text = store::canonical(&note::print(&doc), &path)?;
            let doc = Input::Wcl(text.clone()).parse(&path)?;
            let report = schema
                .validate_note(&c.type_name, &doc, &lookup)
                .map_err(|issues| {
                    Error::Invalid(Invalid {
                        file: path.clone(),
                        issues,
                    })
                })?;
            out.push(Rendered {
                change: c.clone(),
                text: Some(text),
                doc: Some((doc, report)),
            });
        }
        Ok(out)
    }
}

struct Rendered {
    change: NoteChange,
    text: Option<String>,
    doc: Option<(Doc, NoteReport)>,
}

/// Note and asset existence as validation sees it: the files on disk, with this write's own
/// creates added and its deletes removed.
struct Lookup<'a> {
    writer: &'a Writer,
    pending: HashMap<String, String>,
    deleted: HashSet<String>,
}

impl NoteLookup for Lookup<'_> {
    fn note_type(&self, id: &str) -> Option<String> {
        if self.deleted.contains(id) {
            return None;
        }
        if let Some(t) = self.pending.get(id) {
            return Some(t.clone());
        }
        self.writer.locate(id).ok().flatten()
    }

    fn asset_exists(&self, id: &str) -> bool {
        self.writer.find_asset(id).is_some() || self.writer.hooks.asset_known(id)
    }

    fn asset_filename(&self, id: &str) -> Option<String> {
        self.writer.find_asset(id).map(|a| a.filename)
    }
}

/// `notes/<type>/<id>.wcl`.
/// The vault path of type `name`'s definition.
pub fn type_path(name: &str) -> String {
    format!("types/{name}.wcl")
}

fn inner_has_type(inner: &Inner, name: &str) -> bool {
    inner.schema.type_def(name).is_some()
}

fn pairs(v: &[(String, String)]) -> Vec<(&str, &str)> {
    v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
}

pub fn note_path(type_name: &str, id: &str) -> String {
    format!("notes/{type_name}/{id}.wcl")
}

/// A client may not change a computed field: on a new note it must be absent, and on an update it
/// must hold what the note already holds (D3).
fn computed_writes(ty: &TypeDef, doc: &Doc, old: Option<&Doc>) -> Vec<Issue> {
    let block = |d: &'_ Doc, name: &str| d.blocks.iter().find(|b| b.label() == Some(name)).cloned();
    let mut issues = Vec::new();
    for f in ty.fields.iter().filter(|f| f.computed) {
        let changed = if f.kind.is_compound() {
            let new = block(doc, &f.name);
            new.is_some() && new != old.and_then(|d| block(d, &f.name))
        } else {
            let new = doc.field(&f.name);
            new.is_some() && new != old.and_then(|d| d.field(&f.name))
        };
        if changed {
            issues.push(Issue::new(&f.name, "is computed: only handlers write it"));
        }
    }
    issues
}

fn created_of(doc: &Doc) -> Option<String> {
    doc.field("created")
        .and_then(schema::Value::as_str)
        .filter(|s| schema::is_utc_datetime(s))
        .map(str::to_owned)
}

fn set_field(doc: &mut Doc, name: &str, value: String) {
    for (n, v) in &mut doc.fields {
        if n == name {
            *v = schema::Value::Str(value);
            return;
        }
    }
}

/// The submitted note, carrying every other change of the write in `also`.
fn first(saved: Vec<Saved>) -> Saved {
    let mut saved = saved.into_iter();
    let mut first = saved
        .next()
        .expect("a write changes at least the submitted note");
    first.also = saved.collect();
    first
}

fn describe_referrers(referrers: &[Referrer]) -> String {
    referrers
        .iter()
        .map(|r| format!("{} {} (field {})", r.type_name, r.id, r.field))
        .collect::<Vec<_>>()
        .join(", ")
}
