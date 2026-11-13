//! One sync round (git-sync D3, D7 to D17, D21, D23, D25 to D28): fetch every instance branch,
//! import from each other instance's published set, archive what left it, publish this
//! instance's own set, and force-push it as one commit when it changed.
//!
//! - **Import.** Each other branch's manifest is decrypted (a [`Error::KeyMismatch`] or a newer
//!   format skips the branch), its notes are indexed in a throwaway index under its own
//!   definitions, and the import query runs over that index with every page followed. The
//!   selected set is the matches plus every published note this vault created (D10); a note
//!   this vault deleted after syncing it is skipped (D17). Missing definitions are created and
//!   never overwritten (D15), missing assets are written under their ids, and links to notes this
//!   vault does not have are dropped (D16). A note new here is created; one unchanged here since
//!   its base takes the published version; one changed on both sides is merged field by field,
//!   a clash going to the note's origin (D8), with a notification naming the fields (D9).
//! - **Archive.** A note imported from another instance that no instance publishes any more, or
//!   that the import query no longer selects, is archived, and remembered as archived by sync;
//!   when it is selected again it is unarchived (D3, D26, D29). A note the user archived stays
//!   archived.
//! - **Export.** The export query's notes, plus the imported notes changed here, go out
//!   encrypted, with their assets and the definitions they use. An imported note goes out as
//!   its published base with only the fields changed here applied over it, so fields and links
//!   this vault could not hold pass through (D27).
//! - **Push.** Nothing is pushed when the tree equals the branch's; otherwise one parentless
//!   commit is force-pushed (D11). A published set over `max_bytes` is refused (D25).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use schema::{Doc, FieldDef, FieldKind, NoteLookup, Schema, Value};
use serde::{Deserialize, Serialize};
use state::{Author, GitsyncNote, NewNotification};
use sync::{Side, merge_preferring};
use writer::{NoteState, Writer};

use crate::layout::PART_BYTES;
use crate::manifest::{AssetEntry, DefEntry, Instance, Manifest, NoteEntry};
use crate::settings::{AuthKind, Secrets, Settings};
use crate::{Auth, Error, Git, Layout, Result, SyncKey, clip, pack};

/// The stamps every save writes, which take no part in telling whether a note changed.
const STAMPS: [&str; 3] = ["created", "updated", "archived"];

/// Where a round keeps its clone and which programs it runs.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `.vault/gitsync`: the bare clone in `repo/`, and the round's scratch indexes.
    pub dir: PathBuf,
    /// The `git` binary, `git` from `PATH` when `None`.
    pub git_bin: Option<PathBuf>,
    /// The `ssh` binary, `ssh` from `PATH` when `None` (the `VAULT_GITSYNC_SSH` seam).
    pub ssh_bin: Option<PathBuf>,
    /// The largest asset part, [`PART_BYTES`] outside tests.
    pub part_bytes: usize,
}

impl Paths {
    /// The paths of `writer`'s vault, with the default programs and part size.
    pub fn of(writer: &Writer) -> Paths {
        Paths {
            dir: writer.vault().server_dir().join("gitsync"),
            git_bin: None,
            ssh_bin: None,
            part_bytes: PART_BYTES,
        }
    }

    pub fn repo(&self) -> PathBuf {
        self.dir.join("repo")
    }
}

/// A branch the round did not import from, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub branch: String,
    /// The instance's name, when its manifest could be read.
    pub instance: Option<String>,
    pub reason: String,
}

/// A note both sides changed in the same field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clash {
    pub id: String,
    pub title: String,
    /// The instance whose values were kept: the note's origin (D8).
    pub kept: String,
    pub fields: Vec<String>,
}

/// What a round did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    /// Whether the branch was pushed: false when its tree did not change.
    pub pushed: bool,
    /// The notes created, updated with another instance's version, and merged.
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub merged: Vec<String>,
    pub clashes: Vec<Clash>,
    pub archived: Vec<String>,
    pub unarchived: Vec<String>,
    pub skipped_branches: Vec<Skipped>,
    pub warnings: Vec<String>,
    /// The size of the published blobs, the manifest left out.
    pub bytes: u64,
    /// How many notes this instance publishes.
    pub published: usize,
    /// This instance's branch.
    pub branch: String,
    /// The SSH host lines learned under `accept-new`, for the caller to pin (D19).
    #[serde(skip)]
    pub learned_hosts: Option<String>,
}

impl Report {
    /// Whether the round changed anything here or in the repo.
    pub fn changed(&self) -> bool {
        self.pushed
            || !self.created.is_empty()
            || !self.updated.is_empty()
            || !self.merged.is_empty()
            || !self.clashes.is_empty()
            || !self.archived.is_empty()
            || !self.unarchived.is_empty()
    }

    /// A one-line summary for the activity log.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (n, word) in [
            (self.created.len(), "imported"),
            (self.updated.len(), "updated"),
            (self.merged.len(), "merged"),
            (self.clashes.len(), "clashed"),
            (self.archived.len(), "archived"),
            (self.unarchived.len(), "unarchived"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {word}"));
            }
        }
        if self.pushed {
            parts.push(format!("published {} notes", self.published));
        }
        if !self.skipped_branches.is_empty() {
            parts.push(format!("{} branches skipped", self.skipped_branches.len()));
        }
        if !self.warnings.is_empty() {
            parts.push(format!("{} warnings", self.warnings.len()));
        }
        if parts.is_empty() {
            "Git sync: nothing changed".to_owned()
        } else {
            format!("Git sync: {}", parts.join(", "))
        }
    }
}

fn fail(message: impl std::fmt::Display) -> Error {
    Error::Sync(message.to_string())
}

/// Whether `url` names a repo on this machine, which needs no sign-in.
fn is_local(url: &str) -> bool {
    url.starts_with("file://") || url.starts_with('/')
}

/// The git runner for the connection.
pub fn runner(settings: &Settings, secrets: &Secrets, paths: &Paths) -> Result<Git> {
    let auth = if is_local(&settings.remote_url) {
        Auth::None
    } else {
        match settings.auth {
            AuthKind::Https => match &secrets.token {
                Some(token) => Auth::Https {
                    username: settings.https_username.clone(),
                    token: token.clone(),
                },
                None => Auth::None,
            },
            AuthKind::Ssh => Auth::Ssh {
                private_key: secrets
                    .ssh_key
                    .clone()
                    .ok_or_else(|| fail("no SSH key: make or paste one in Settings > Sync"))?,
                known_hosts: secrets.known_hosts.clone(),
            },
        }
    };
    std::fs::create_dir_all(&paths.dir)?;
    let mut git = Git::new(paths.repo(), auth)?;
    if let Some(bin) = &paths.git_bin {
        git = git.with_bin(bin);
    }
    if let Some(bin) = &paths.ssh_bin {
        git = git.with_ssh_bin(bin);
    }
    Ok(git)
}

/// Remove `branch` from the remote: the branch this instance pushed under a key it no longer
/// has (D14).
pub fn remove_branch(
    settings: &Settings,
    secrets: &Secrets,
    paths: &Paths,
    branch: &str,
) -> Result<()> {
    let git = runner(settings, secrets, paths)?;
    git.ensure_bare_clone(&settings.remote_url)?;
    let refspec = format!(":refs/heads/{branch}");
    let out = git
        .command(&["push", "--force", "origin", refspec.as_str()])
        .output()?;
    if !out.status.success() {
        return Err(Error::Git {
            command: "push".into(),
            status: out.status.to_string(),
            stderr: git.redact(&String::from_utf8_lossy(&out.stderr)),
        });
    }
    Ok(())
}

/// `notes/<type>/<id>.wcl`, the name diagnostics use for a note.
fn note_path(type_name: &str, id: &str) -> String {
    format!("notes/{type_name}/{id}.wcl")
}

fn parse_note(type_name: &str, id: &str, wcl: &str) -> Result<Doc> {
    Doc::parse(wcl, &note_path(type_name, id)).map_err(|e| fail(e.to_string()))
}

/// The WCL of `doc`, canonical.
fn print(type_name: &str, id: &str, doc: &Doc) -> Result<String> {
    store::canonical(&writer::print_doc(doc), &note_path(type_name, id))
        .map_err(|e| fail(format!("note {id}: {e}")))
}

fn field<'a>(doc: &'a Doc, name: &str) -> Option<&'a Value> {
    doc.field(name)
}

/// Set top-level field `name` of `doc` to `value`, or remove it for `None`.
fn set_field(doc: &mut Doc, name: &str, value: Option<Value>) {
    match (doc.fields.iter().position(|(n, _)| n == name), value) {
        (Some(i), Some(v)) => doc.fields[i].1 = v,
        (Some(i), None) => {
            doc.fields.remove(i);
        }
        (None, Some(v)) => doc.fields.push((name.to_owned(), v)),
        (None, None) => {}
    }
}

/// `doc` without its stamps, to compare what the note holds.
fn content(doc: &Doc) -> Doc {
    let mut d = doc.clone();
    d.fields.retain(|(n, _)| !STAMPS.contains(&n.as_str()));
    d
}

/// The later of two `updated` stamps.
fn later(a: Option<&Value>, b: Option<&Value>) -> Option<Value> {
    match (a.and_then(Value::as_str), b.and_then(Value::as_str)) {
        (Some(x), Some(y)) if x >= y => a.cloned(),
        (_, Some(_)) => b.cloned(),
        _ => a.cloned(),
    }
}

/// `doc` with every reference to a note `present` says this vault lacks dropped (D16). Only
/// reference fields `type_def` declares are touched.
fn strip_links(
    doc: &Doc,
    type_def: Option<&schema::TypeDef>,
    present: &dyn Fn(&str) -> bool,
) -> Doc {
    let Some(ty) = type_def else {
        return doc.clone();
    };
    let mut out = doc.clone();
    out.fields.retain_mut(|(name, value)| {
        let Some(f) = ty.field(name) else {
            return true;
        };
        if !matches!(f.kind, FieldKind::Reference(_)) {
            return true;
        }
        match value {
            Value::Str(id) => present(id),
            Value::List(items) => {
                items.retain(|v| v.as_str().is_none_or(present));
                true
            }
            _ => true,
        }
    });
    out
}

/// Add the types and taxonomies field `f` uses to `types` and `taxonomies`.
fn field_uses(f: &FieldDef, types: &mut Vec<String>, taxonomies: &mut BTreeSet<String>) {
    match &f.kind {
        FieldKind::Taxonomy { taxonomy } => {
            taxonomies.insert(taxonomy.clone());
        }
        FieldKind::Reference(r) => types.extend(r.target.clone()),
        FieldKind::List { item } => field_uses(item, types, taxonomies),
        FieldKind::Dict { keys, values } => {
            for k in keys {
                field_uses(k, types, taxonomies);
            }
            if let schema::DictValues::Of(d) = values {
                field_uses(d, types, taxonomies);
            }
        }
        _ => {}
    }
}

/// The types reachable from `roots` through reference fields, and every taxonomy they use, as
/// `schema` describes them.
fn definitions_used(
    schema: &Schema,
    roots: impl IntoIterator<Item = String>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut types = BTreeSet::new();
    let mut taxonomies = BTreeSet::new();
    let mut stack: Vec<String> = roots.into_iter().collect();
    while let Some(t) = stack.pop() {
        let Some(def) = schema.type_def(&t) else {
            continue;
        };
        if !types.insert(t) {
            continue;
        }
        for f in def.fields.iter().filter(|f| !f.implied) {
            field_uses(f, &mut stack, &mut taxonomies);
        }
    }
    (types, taxonomies)
}

/// Every id `q` selects over `ctx`, every page followed (D23).
fn all_ids(q: &query::Query, ctx: &query::Context<'_>) -> query::Result<Vec<String>> {
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = query::run(
            q,
            ctx,
            &query::PageRequest {
                cursor,
                size: query::MAX_PAGE,
            },
        )?;
        ids.extend(page.rows.into_iter().map(|r| r.id));
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(ids),
        }
    }
}

/// A query text, `None` when empty.
fn parse_query(text: &str, which: &str) -> Result<Option<query::Query>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    query::parse(text)
        .map(Some)
        .map_err(|e| fail(format!("the {which} query does not parse: {}", e.message)))
}

/// One published note of another instance.
struct Incoming {
    entry: NoteEntry,
    type_name: String,
    wcl: String,
    doc: Doc,
}

/// Another instance's branch, read.
struct Branch {
    peer: String,
    peer_name: String,
    manifest: Manifest,
    /// Blob ids by path.
    oids: HashMap<String, String>,
    notes: BTreeMap<String, Incoming>,
    types: BTreeMap<String, String>,
    taxonomies: BTreeMap<String, String>,
    schema: Schema,
    /// The notes to import from this branch, in id order.
    selected: BTreeSet<String>,
    /// Whether the import query failed against this branch, so its notes are kept as if
    /// selected rather than archived.
    unselectable: bool,
}

impl Branch {
    fn author(&self) -> Author {
        Author::Task(format!("gitsync {}", self.peer_name))
    }
}

/// Validation of a throwaway index's notes: the published set's notes exist, and every asset
/// does, since the real check happens at the import write.
struct Published<'a>(&'a HashMap<String, String>);

impl NoteLookup for Published<'_> {
    fn note_type(&self, id: &str) -> Option<String> {
        self.0.get(id).cloned()
    }

    fn asset_exists(&self, _id: &str) -> bool {
        true
    }
}

/// What one round works with.
struct Round<'a> {
    w: &'a Writer,
    settings: &'a Settings,
    key: &'a SyncKey,
    paths: &'a Paths,
    git: Git,
    me: String,
    report: Report,
}

/// Run one round. Only one runs at a time; the caller holds that lock.
pub fn run_round(
    w: &Writer,
    settings: &Settings,
    secrets: &Secrets,
    paths: &Paths,
) -> Result<Report> {
    let key = secrets
        .key
        .as_ref()
        .ok_or_else(|| fail("no sync key: generate one or paste one in Settings > Sync"))?;
    if settings.remote_url.trim().is_empty() {
        return Err(fail("no remote URL is set"));
    }
    let me = w.state().vault_id()?;
    let git = runner(settings, secrets, paths)?;
    git.ensure_bare_clone(&settings.remote_url)?;
    git.fetch_instance_branches()?;
    let mut round = Round {
        w,
        settings,
        key,
        paths,
        git,
        report: Report {
            branch: Layout::new(key, &me).branch(),
            ..Report::default()
        },
        me,
    };
    let import = parse_query(&settings.import_query, "import")?;
    let mut branches = Vec::new();
    for (name, _) in round.git.fetched_branches()? {
        if name == round.report.branch {
            continue;
        }
        match round.read_branch(&name, import.as_ref()) {
            Ok(b) => branches.push(b),
            Err(Skip { instance, reason }) => round.report.skipped_branches.push(Skipped {
                branch: name,
                instance,
                reason,
            }),
        }
    }
    round.import(&branches)?;
    round.archive(&branches)?;
    round.export()?;
    if matches!(round.git.auth(), Auth::Ssh { .. }) && secrets.known_hosts.trim().is_empty() {
        round.report.learned_hosts = round.git.learned_host_line();
    }
    Ok(round.report)
}

/// Why a branch was skipped.
struct Skip {
    instance: Option<String>,
    reason: String,
}

impl Skip {
    fn new(reason: impl std::fmt::Display) -> Skip {
        Skip {
            instance: None,
            reason: reason.to_string(),
        }
    }
}

impl Round<'_> {
    fn author(&self) -> Author {
        Author::Task(format!("gitsync {}", self.settings.name))
    }

    /// Read branch `name`: its manifest, definitions and notes, and which notes to import.
    fn read_branch(
        &mut self,
        name: &str,
        import: Option<&query::Query>,
    ) -> std::result::Result<Branch, Skip> {
        let layout =
            Layout::for_branch(self.key, name).ok_or_else(|| Skip::new("not a tv- branch"))?;
        let tree = self
            .git
            .read_tree(&format!("refs/remotes/origin/{name}"))
            .map_err(Skip::new)?;
        let oids: HashMap<String, String> = tree
            .into_iter()
            .filter(|e| e.kind == "blob")
            .map(|e| (e.path, e.oid))
            .collect();
        let manifest_oid = oids
            .get(&layout.manifest())
            .ok_or_else(|| Skip::new("the branch has no manifest"))?;
        let blob = self.git.cat_blob(manifest_oid).map_err(Skip::new)?;
        let manifest = match Manifest::open(self.key, &layout, &blob) {
            Ok(m) => m,
            Err(Error::KeyMismatch) => {
                return Err(Skip::new(
                    "key mismatch: the branch was written with a different sync key",
                ));
            }
            Err(e) => return Err(Skip::new(e)),
        };
        let peer = manifest.instance.vault_id.clone();
        let named = |reason: String| Skip {
            instance: Some(manifest.instance.name.clone()),
            reason,
        };
        if peer == self.me {
            return Err(named(
                "the branch is this vault's own, written under another sync key or by a copy"
                    .into(),
            ));
        }
        if Layout::new(self.key, &peer).folder() != layout.folder() {
            return Err(named(
                "the branch does not belong to the instance it names".into(),
            ));
        }

        let open_defs = |defs: &[DefEntry], taxonomy: bool| -> std::result::Result<BTreeMap<String, String>, Skip> {
            let found: Vec<(&DefEntry, &str)> = defs
                .iter()
                .filter_map(|d| oids.get(&d.path).map(|o| (d, o.as_str())))
                .collect();
            let blobs = self
                .git
                .cat_blobs(&found.iter().map(|(_, o)| *o).collect::<Vec<_>>())
                .map_err(Skip::new)?;
            let mut out = BTreeMap::new();
            for ((d, _), blob) in found.into_iter().zip(blobs) {
                let path = if taxonomy { layout.taxonomy(&d.name) } else { layout.type_def(&d.name) };
                if d.path != path {
                    return Err(Skip::new("definition path does not match its identity"));
                }
                let text = self.key.open(&path, &blob).map_err(Skip::new)?;
                let text = String::from_utf8(text)
                    .map_err(|_| Skip::new(format!("definition {} is not text", d.name)))?;
                out.insert(d.name.clone(), text);
            }
            Ok(out)
        };
        let types = open_defs(&manifest.types, false)?;
        let taxonomies = open_defs(&manifest.taxonomies, true)?;
        let schema = {
            let t: Vec<(&str, &str)> = types
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str()))
                .collect();
            let x: Vec<(&str, &str)> = taxonomies
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str()))
                .collect();
            Schema::from_sources(&t, &x)
                .map_err(|e| named(format!("its definitions do not load: {e}")))?
        };

        let found: Vec<(&NoteEntry, &str)> = manifest
            .notes
            .iter()
            .filter_map(|n| oids.get(&n.path).map(|o| (n, o.as_str())))
            .collect();
        let blobs = self
            .git
            .cat_blobs(&found.iter().map(|(_, o)| *o).collect::<Vec<_>>())
            .map_err(Skip::new)?;
        let mut notes = BTreeMap::new();
        for ((entry, _), blob) in found.into_iter().zip(blobs) {
            let path = layout.note(&entry.id);
            if entry.path != path {
                return Err(Skip::new("note path does not match its identity"));
            }
            let opened = pack::open_note(self.key, &path, &blob).and_then(|(t, wcl)| {
                if t != entry.note_type || pack::sha256_hex(&wcl) != entry.hash {
                    return Err(Error::Corrupt { id: entry.id.clone(), detail: "manifest note type or hash mismatch".into() });
                }
                let wcl =
                    String::from_utf8(wcl).map_err(|_| Error::Malformed("not UTF-8".into()))?;
                Ok((t, wcl))
            });
            let (type_name, wcl) = match opened {
                Ok(n) => n,
                Err(e) => {
                    return Err(Skip::new(format!(
                        "{}: note {}: {e}",
                        manifest.instance.name, entry.id
                    )));
                }
            };
            // Transcripts are local runtime records, never peer-authored content.
            if type_name == writer::AGENT_THREAD_TYPE
                || entry.note_type == writer::AGENT_THREAD_TYPE
            {
                continue;
            }
            match parse_note(&type_name, &entry.id, &wcl) {
                Ok(doc) => {
                    notes.insert(
                        entry.id.clone(),
                        Incoming {
                            entry: entry.clone(),
                            type_name,
                            wcl,
                            doc,
                        },
                    );
                }
                Err(e) => self.report.warnings.push(format!(
                    "{}: note {}: {e}",
                    manifest.instance.name, entry.id
                )),
            }
        }

        let mut selected: BTreeSet<String> = notes
            .values()
            .filter(|n| n.entry.origin == self.me)
            .map(|n| n.entry.id.clone())
            .collect();
        let mut unselectable = false;
        if let Some(q) = import {
            match self.select(&schema, &notes, q) {
                Ok(ids) => selected.extend(ids.into_iter().filter(|id| notes.contains_key(id))),
                // The instance has no such type: nothing of it matches.
                Err(query::Error::UnknownType(_)) => {}
                Err(e) => {
                    unselectable = true;
                    self.report.warnings.push(format!(
                        "the import query cannot run against {}: {e}",
                        manifest.instance.name
                    ));
                }
            }
        }
        Ok(Branch {
            peer,
            peer_name: manifest.instance.name.clone(),
            manifest,
            oids,
            notes,
            types,
            taxonomies,
            schema,
            selected,
            unselectable,
        })
    }

    /// The ids `q` selects among `notes`, indexed in a throwaway index under `schema`
    /// (notes/02): `set_schema` and `update`, never `rebuild`, which would drop a note whose
    /// links point outside the published set.
    fn select(
        &self,
        schema: &Schema,
        notes: &BTreeMap<String, Incoming>,
        q: &query::Query,
    ) -> query::Result<Vec<String>> {
        let tmp = tempfile::Builder::new()
            .prefix("import-")
            .tempdir_in(&self.paths.dir)
            .map_err(|e| query::Error::Invalid(e.to_string()))?;
        let mut index =
            index::Index::open_at(&tmp.path().join("index.db"), &tmp.path().join("search"))?;
        index.set_schema(schema)?;
        let types: HashMap<String, String> = notes
            .values()
            .map(|n| (n.entry.id.clone(), n.type_name.clone()))
            .collect();
        let lookup = Published(&types);
        let updates: Vec<index::Update> = notes
            .values()
            .filter(|n| schema.type_def(&n.type_name).is_some())
            .map(|n| index::Update::Put {
                id: n.entry.id.clone(),
                type_name: n.type_name.clone(),
                doc: n.doc.clone(),
                report: schema
                    .validate_note(&n.type_name, &n.doc, &lookup)
                    .unwrap_or_default(),
            })
            .collect();
        index.update(schema, &updates)?;
        all_ids(q, &query::Context::new(&index, schema))
    }

    /// Whether note `id` is one this vault deleted after syncing it (D17).
    fn tombstoned(&self, id: &str) -> Result<bool> {
        if matches!(self.w.note_state(id)?, NoteState::Live { .. }) {
            return Ok(false);
        }
        Ok(!self.w.state().gitsync_rows_for_note(id)?.is_empty())
    }

    /// Import every branch's selected notes.
    fn import(&mut self, branches: &[Branch]) -> Result<()> {
        // The notes this round creates, so links between them are kept.
        let mut arriving = BTreeSet::new();
        for b in branches {
            for id in &b.selected {
                if matches!(self.w.note_state(id)?, NoteState::Absent)
                    && self.w.state().gitsync_rows_for_note(id)?.is_empty()
                {
                    arriving.insert(id.clone());
                }
            }
        }
        for b in branches {
            self.import_definitions(b)?;
            for id in &b.selected {
                if self.tombstoned(id)? {
                    continue;
                }
                if let Err(e) = self.import_note(b, &b.notes[id], &arriving) {
                    self.report
                        .warnings
                        .push(format!("{}: note {id}: {e}", b.peer_name));
                }
            }
        }
        Ok(())
    }

    /// Create the types and taxonomies the selected notes use that this vault lacks, never
    /// changing one it has (D15).
    fn import_definitions(&mut self, b: &Branch) -> Result<()> {
        let (types, taxonomies) = definitions_used(
            &b.schema,
            b.selected.iter().map(|id| b.notes[id].type_name.clone()),
        );
        let local = self.w.schema();
        for name in taxonomies.iter().filter(|t| local.taxonomy(t).is_none()) {
            let Some(source) = b.taxonomies.get(name) else {
                continue;
            };
            if let Err(e) = self.w.put_taxonomy_if_missing(name, source, &b.author()) {
                self.report.warnings.push(format!(
                    "{}: taxonomy {name} could not be created: {e}",
                    b.peer_name
                ));
            }
        }
        // A type can name another not created yet: retry until nothing more goes in.
        let mut pending: Vec<&String> = types
            .iter()
            .filter(|t| local.type_def(t).is_none() && b.types.contains_key(*t))
            .collect();
        loop {
            let mut failed = Vec::new();
            let before = pending.len();
            for name in pending {
                if let Err(e) = self.w.put_type_if_missing(name, &b.types[name]) {
                    failed.push((name, e));
                }
            }
            if failed.is_empty() {
                break;
            }
            if failed.len() == before {
                for (name, e) in failed {
                    self.report.warnings.push(format!(
                        "{}: type {name} could not be created: {e}",
                        b.peer_name
                    ));
                }
                break;
            }
            pending = failed.into_iter().map(|(n, _)| n).collect();
        }
        Ok(())
    }

    /// Write the assets `n` names that this vault lacks, under their ids.
    fn import_assets(&mut self, b: &Branch, n: &Incoming) {
        for id in &n.entry.assets {
            if self.w.asset_info(id).is_ok() {
                continue;
            }
            let Some(asset) = b.manifest.assets.iter().find(|a| &a.id == id) else {
                continue;
            };
            let written = (|| -> Result<()> {
                let oids: Vec<&str> = asset
                    .parts
                    .iter()
                    .map(|p| {
                        b.oids
                            .get(p)
                            .map(String::as_str)
                            .ok_or_else(|| Error::Corrupt {
                                id: id.clone(),
                                detail: "a part is missing".into(),
                            })
                    })
                    .collect::<Result<_>>()?;
                let parts = self.git.cat_blobs(&oids)?;
                let layout = Layout::new(self.key, &b.peer);
                if asset.parts.iter().enumerate().any(|(i, p)| *p != layout.asset_part(id, i)) {
                    return Err(Error::Malformed("asset part path does not match its identity".into()));
                }
                let bytes = pack::join(self.key, &layout, parts, id, asset.size, &asset.sha256)?;
                self.w.put_asset_with_id(id, &asset.filename, bytes)?;
                Ok(())
            })();
            if let Err(e) = written {
                self.report
                    .warnings
                    .push(format!("{}: asset {id}: {e}", b.peer_name));
            }
        }
    }

    /// Import one selected note of branch `b`.
    fn import_note(&mut self, b: &Branch, n: &Incoming, arriving: &BTreeSet<String>) -> Result<()> {
        let id = n.entry.id.as_str();
        let w = self.w;
        let state = w.state();
        let rows = state.gitsync_rows_for_note(id)?;
        let row = rows.iter().find(|r| r.peer_vault_id == b.peer);
        let local_schema = w.schema();
        let present = |target: &str| {
            arriving.contains(target) || matches!(w.note_state(target), Ok(NoteState::Live { .. }))
        };
        let strip = |doc: &Doc, type_name: &str| {
            strip_links(doc, local_schema.type_def(type_name), &present)
        };
        let store_base = |archived_by_sync: bool| {
            state.gitsync_put(&GitsyncNote {
                note_id: id.to_owned(),
                peer_vault_id: b.peer.clone(),
                origin: n.entry.origin.clone(),
                base: n.wcl.as_bytes().to_vec(),
                base_hash: n.entry.hash.clone(),
                archived_by_sync,
            })
        };
        let author = b.author();
        match w.note_state(id)? {
            NoteState::Trashed => Ok(()),
            NoteState::Absent => {
                if local_schema.type_def(&n.type_name).is_none() {
                    return Err(fail(format!("this vault has no type {}", n.type_name)));
                }
                self.import_assets(b, n);
                let text = print(&n.type_name, id, &strip(&n.doc, &n.type_name))?;
                w.create_remote(id, &n.type_name, &text, &author)?;
                store_base(false)?;
                self.report.created.push(id.to_owned());
                Ok(())
            }
            NoteState::Live { type_name, version } => {
                if type_name == writer::AGENT_THREAD_TYPE {
                    return Ok(());
                }
                if row.is_some_and(|r| r.base_hash == n.entry.hash) {
                    // Unchanged there since it was last taken.
                    return Ok(());
                }
                let base_row = row
                    .or_else(|| rows.iter().find(|r| r.peer_vault_id == n.entry.origin))
                    .or_else(|| rows.iter().find(|r| r.peer_vault_id == self.me))
                    .or(rows.first());
                let local = w.read(id)?;
                let base = match base_row {
                    Some(r) => {
                        let text = String::from_utf8_lossy(&r.base).into_owned();
                        Some(strip(&parse_note(&type_name, id, &text)?, &type_name))
                    }
                    None => None,
                };
                let remote = strip(&n.doc, &n.type_name);
                if local_schema.type_def(&n.type_name).is_none() {
                    return Err(fail(format!("this vault has no type {}", n.type_name)));
                }
                self.import_assets(b, n);
                let unchanged_here = base
                    .as_ref()
                    .is_some_and(|base| content(base) == content(&local.doc));
                let take = unchanged_here || content(&remote) == content(&local.doc);
                let (mut doc, out_type, clashes) = if take {
                    (remote, n.type_name.clone(), Vec::new())
                } else {
                    let winner = if n.entry.origin == self.me {
                        Side::Ours
                    } else {
                        Side::Theirs
                    };
                    let r = merge_preferring(base.as_ref(), &local.doc, &remote, winner);
                    let out_type = match winner {
                        Side::Ours => type_name.clone(),
                        Side::Theirs => n.type_name.clone(),
                    };
                    let mut doc = r.doc;
                    set_field(
                        &mut doc,
                        "updated",
                        later(field(&local.doc, "updated"), field(&n.doc, "updated")),
                    );
                    (doc, out_type, r.clashes)
                };
                // The archive state here is this vault's own (D26).
                set_field(&mut doc, "archived", field(&local.doc, "archived").cloned());
                let text = print(&out_type, id, &doc)?;
                if text != local.wcl {
                    w.replace_remote(id, &out_type, &text, &version, &author)?;
                    if take {
                        self.report.updated.push(id.to_owned());
                    } else {
                        self.report.merged.push(id.to_owned());
                    }
                }
                store_base(row.is_some_and(|r| r.archived_by_sync))?;
                if !clashes.is_empty() {
                    self.clash(id, &local.doc, b, &n.entry.origin, clashes);
                }
                Ok(())
            }
        }
    }

    /// Record that note `id` clashed in `fields`, and tell the user when this vault's values
    /// lost: the instance that lost is the one notified, once (D9).
    fn clash(&mut self, id: &str, doc: &Doc, b: &Branch, origin: &str, fields: Vec<String>) {
        let title = field(doc, "title")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_owned();
        let (kept, lost) = if origin == self.me {
            (self.settings.name.clone(), b.peer_name.clone())
        } else {
            (b.peer_name.clone(), self.settings.name.clone())
        };
        let n = NewNotification {
            title: clip(
                &format!("Sync clash in \u{2018}{title}\u{2019}"),
                state::MAX_TITLE,
            ),
            body: Some(clip(
                &format!(
                    "{} changed on both {kept} and {lost}. {kept}'s values were kept, as the note \
                     came from there; {lost}'s version is in the note's history.",
                    fields.join(", ")
                ),
                state::MAX_BODY,
            )),
            note_id: Some(id.to_owned()),
            url: None,
            source: "system".to_owned(),
        };
        if origin != self.me
            && let Err(e) = self.w.state().add_notification(&n)
        {
            eprintln!("gitsync: recording the clash notification for {id}: {e}");
        }
        self.report.clashes.push(Clash {
            id: id.to_owned(),
            title,
            kept,
            fields,
        });
    }

    /// Archive the imported notes no instance publishes any more or the import query no longer
    /// selects, and unarchive those archived by sync that are selected again (D3, D26, D29). A
    /// branch the import query failed against keeps every note it publishes.
    fn archive(&mut self, branches: &[Branch]) -> Result<()> {
        let state = self.w.state();
        let author = self.author();
        let selected: BTreeSet<&str> = branches
            .iter()
            .flat_map(|b| b.selected.iter().map(String::as_str))
            .collect();
        let kept: BTreeSet<&str> = branches
            .iter()
            .filter(|b| b.unselectable)
            .flat_map(|b| b.notes.keys().map(String::as_str))
            .chain(selected.iter().copied())
            .collect();
        for b in branches {
            for row in state.gitsync_rows_for_peer(&b.peer)? {
                if row.origin == self.me || kept.contains(row.note_id.as_str()) {
                    continue;
                }
                if row.archived_by_sync {
                    continue;
                }
                let Ok(note) = self.w.read(&row.note_id) else {
                    continue;
                };
                if note.type_name == writer::AGENT_THREAD_TYPE || writer::is_archived(&note.doc) {
                    continue;
                }
                self.w
                    .set_archived(&row.note_id, true, Some(&note.version), &author)?;
                state.gitsync_set_archived_by_sync(&row.note_id, &row.peer_vault_id, true)?;
                self.report.archived.push(row.note_id);
            }
        }
        for id in selected {
            let rows = state.gitsync_rows_for_note(id)?;
            let flagged: Vec<&GitsyncNote> = rows.iter().filter(|r| r.archived_by_sync).collect();
            if flagged.is_empty() {
                continue;
            }
            if let Ok(note) = self.w.read(id)
                && writer::is_archived(&note.doc)
            {
                self.w
                    .set_archived(id, false, Some(&note.version), &author)?;
                self.report.unarchived.push(id.to_owned());
            }
            for r in flagged {
                state.gitsync_set_archived_by_sync(id, &r.peer_vault_id, false)?;
            }
        }
        Ok(())
    }

    /// The published form of imported note `local`: `base` with only the fields changed here
    /// applied over it (D27). A field that differs from the base only by the links dropped on
    /// import is the base's.
    fn publish_imported(&self, local: &writer::Note, base: &GitsyncNote) -> Result<String> {
        let id = local.id.as_str();
        let base_text = String::from_utf8_lossy(&base.base).into_owned();
        let base_doc = parse_note(&local.type_name, id, &base_text)?;
        let schema = self.w.schema();
        let present = |t: &str| matches!(self.w.note_state(t), Ok(NoteState::Live { .. }));
        let stripped = strip_links(&base_doc, schema.type_def(&local.type_name), &present);
        if content(&stripped) == content(&local.doc) {
            return Ok(base_text);
        }
        let mut doc = merge_preferring(Some(&stripped), &local.doc, &base_doc, Side::Ours).doc;
        set_field(&mut doc, "updated", field(&local.doc, "updated").cloned());
        print(&local.type_name, id, &doc)
    }

    /// Publish this instance's set and push it when it changed.
    fn export(&mut self) -> Result<()> {
        let state = self.w.state();
        let mut imported: BTreeMap<String, Vec<GitsyncNote>> = BTreeMap::new();
        for row in state.gitsync_imported(&self.me)? {
            imported.entry(row.note_id.clone()).or_default().push(row);
        }
        let base_of = |id: &str| {
            imported.get(id).and_then(|rows| {
                rows.iter()
                    .find(|r| r.peer_vault_id == r.origin)
                    .or(rows.first())
            })
        };
        let export = parse_query(&self.settings.export_query, "export")?;
        let ids: Vec<String> = match &export {
            Some(q) => self
                .w
                .read_index(|index, schema| all_ids(q, &query::Context::new(index, schema)))?,
            None => Vec::new(),
        };

        // id -> (type, canonical WCL, origin)
        let mut out: BTreeMap<String, (String, String, String)> = BTreeMap::new();
        for id in &ids {
            let Ok(note) = self.w.read(id) else { continue };
            if note.type_name == writer::AGENT_THREAD_TYPE {
                continue;
            }
            let entry = match base_of(id) {
                Some(base) => (
                    note.type_name.clone(),
                    self.publish_imported(&note, base)?,
                    base.origin.clone(),
                ),
                None => (note.type_name.clone(), note.wcl.clone(), self.me.clone()),
            };
            out.insert(id.clone(), entry);
        }
        for (id, rows) in &imported {
            if out.contains_key(id) {
                continue;
            }
            let Ok(note) = self.w.read(id) else { continue };
            if note.type_name == writer::AGENT_THREAD_TYPE {
                continue;
            }
            let Some(base) = base_of(id) else { continue };
            let text = self.publish_imported(&note, base)?;
            if text.as_bytes() != base.base {
                out.insert(
                    id.clone(),
                    (note.type_name.clone(), text, rows[0].origin.clone()),
                );
            }
        }

        let layout = Layout::new(self.key, &self.me);
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut manifest = Manifest::new(Instance {
            vault_id: self.me.clone(),
            name: self.settings.name.clone(),
        });
        let mut asset_ids = BTreeSet::new();
        for (id, (type_name, wcl, origin)) in &out {
            let assets = self.w.read_index(|index, _| index.assets_of_note(id))?;
            asset_ids.extend(assets.iter().cloned());
            let hash = pack::sha256_hex(wcl.as_bytes());
            let path = layout.note(id);
            files.insert(
                path.clone(),
                pack::seal_note(self.key, &path, type_name, wcl.as_bytes()),
            );
            if origin == &self.me {
                let row = state.gitsync_get(id, &self.me)?;
                if row.is_none_or(|r| r.base_hash != hash) {
                    state.gitsync_put(&GitsyncNote {
                        note_id: id.clone(),
                        peer_vault_id: self.me.clone(),
                        origin: self.me.clone(),
                        base: wcl.as_bytes().to_vec(),
                        base_hash: hash.clone(),
                        archived_by_sync: false,
                    })?;
                }
            }
            manifest.notes.push(NoteEntry {
                id: id.clone(),
                note_type: type_name.clone(),
                origin: origin.clone(),
                hash,
                path,
                assets,
            });
        }
        for id in &asset_ids {
            let Ok((asset, bytes)) = self.w.asset(id) else {
                continue;
            };
            let parts = pack::split(self.key, &layout, id, &bytes, self.paths.part_bytes);
            let mut paths = Vec::new();
            for (k, part) in parts.into_iter().enumerate() {
                let p = layout.asset_part(id, k);
                files.insert(p.clone(), part);
                paths.push(p);
            }
            manifest.assets.push(AssetEntry {
                id: id.clone(),
                filename: asset.filename,
                size: asset.size,
                sha256: pack::sha256_hex(&bytes),
                parts: paths,
            });
        }
        let schema = self.w.schema();
        let (types, taxonomies) =
            definitions_used(&schema, out.values().map(|(t, _, _)| t.clone()));
        for name in &types {
            let Ok(source) = self.w.read_type(name) else {
                continue;
            };
            let path = layout.type_def(name);
            files.insert(path.clone(), self.key.seal(&path, source.as_bytes()));
            manifest.types.push(DefEntry {
                name: name.clone(),
                path,
            });
        }
        for name in &taxonomies {
            let Ok(source) = self.w.read_taxonomy(name) else {
                continue;
            };
            let path = layout.taxonomy(name);
            files.insert(path.clone(), self.key.seal(&path, source.as_bytes()));
            manifest.taxonomies.push(DefEntry {
                name: name.clone(),
                path,
            });
        }
        manifest.bytes = files.values().map(|b| b.len() as u64).sum();
        self.report.bytes = manifest.bytes;
        self.report.published = manifest.notes.len();
        if manifest.bytes > self.settings.max_bytes {
            return Err(fail(format!(
                "the published set is {}, over the {} limit, so the push was refused: narrow the \
                 export query or raise the limit",
                size(manifest.bytes),
                size(self.settings.max_bytes)
            )));
        }
        if manifest.bytes > self.settings.warn_bytes {
            self.report.warnings.push(format!(
                "the published set is {}, over the {} warning size",
                size(manifest.bytes),
                size(self.settings.warn_bytes)
            ));
        }
        files.insert(layout.manifest(), manifest.seal(self.key));

        let tree = self.git.write_tree(&files)?;
        let branch = layout.branch();
        let current = self.git.tree_of(&format!("refs/remotes/origin/{branch}"))?;
        if current.as_deref() == Some(tree.as_str()) {
            return Ok(());
        }
        let commit = self.git.commit_tree(&tree, "TheVault gitsync")?;
        self.git.force_push(&commit, &branch)?;
        self.report.pushed = true;
        Ok(())
    }
}

/// A byte count for people: `1.5 GB`, `20 MB`, `512 bytes`.
fn size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GB"), (1 << 20, "MB"), (1 << 10, "KB")];
    for (unit, name) in UNITS {
        if bytes >= unit {
            let v = bytes as f64 / unit as f64;
            return if v.fract() == 0.0 {
                format!("{v:.0} {name}")
            } else {
                format!("{v:.1} {name}")
            };
        }
    }
    format!("{bytes} bytes")
}
