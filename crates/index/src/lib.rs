//! The SQLite projection in `.vault/index.db` and the tantivy full-text index.
//!
//! Both are derived from the vault files and nothing else, so either can be deleted and rebuilt
//! (D2, D25). [`Index::rebuild`] reads every file, validates it, indexes what passes and reports
//! what does not (D28). [`Index::update`] applies one commit's notes after the writer has
//! validated and committed them. A rebuild and the same notes applied one at a time produce the
//! same [`Index::dump`].
//!
//! The index records the vault commit generation it is current with (see
//! [`store::Vault::generation`]). [`Index::rebuild`] sets it and [`Index::advance`] moves it one
//! commit at a time, so [`Index::is_current`] tells a server on start whether it may skip the
//! rebuild. A commit whose changes never reached the index leaves it behind for good, and the
//! next start rebuilds.
//!
//! `index.db` holds these tables, which `query` reads:
//!
//! | Table | One row per |
//! |-------|-------------|
//! | `notes` | Note: id, type, path, title, created, updated, both again as Unix time, flagged, archived, and its fields as JSON |
//! | `field_values` | Scalar inside a field value, including `title`, `created` and `updated`, with the type of its note |
//! | `links` | Note reference in a field, or mention in inline text, from `src` to `dst` |
//! | `asset_refs` | Asset a note names: in an asset field, or in the `asset` item of a block at any depth |
//! | `terms` | Taxonomy term, with its parent, depth and display position |
//! | `term_ancestors` | Pair of a term and itself or a term above it, for `under()` |
//! | `failures` | Vault file the last rebuild could not index, with its diagnostic |
//! | `meta` | Fact about the index itself: `generation` |
//!
//! The connection also has three SQL functions for `query`: `vault_contains(text, part)`, true
//! when `text` lowercased holds `part`, which the caller lowercases, `vault_same(text, lower)`,
//! true when `text` lowercased is `lower`, for `has_ci()`, and `vault_blank(text)`, true when
//! `text` is empty or only white space. All follow Rust's Unicode rules, as the note by note
//! evaluator does.

mod asset_text;
mod graph;
mod pdf_repair;
mod project;
mod rebuild;
mod search;
mod sql;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use schema::{Doc, NoteReport, Schema};

pub use asset_text::{
    EXTRACT_TIMEOUT, Extracted, MAX_PDF_BYTES, MAX_RUNNING, MAX_TEXT, StoredText, extract_pdf_text,
    is_pdf,
};
pub use graph::{Graph, GraphEdge, GraphNode};
pub use project::mentions;
pub use rebuild::{Failure, RebuildReport};
pub use search::{Hit, SearchPage};
pub use vocab::Archived;

/// Errors from the index. A file that fails validation is not an error: a rebuild reports it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("index.db: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("full-text index: {0}")]
    Search(#[from] tantivy::TantivyError),
    #[error("full-text index: {0}")]
    SearchOpen(#[from] tantivy::directory::error::OpenDirectoryError),
    #[error("full-text query: {0}")]
    SearchQuery(#[from] tantivy::query::QueryParserError),
    #[error(transparent)]
    Store(#[from] store::Error),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("note {id}: there is no type {type_name:?}")]
    UnknownType { id: String, type_name: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// One note change to apply to the index.
#[derive(Debug, Clone)]
pub enum Update {
    /// A note that was created or changed. `doc` is the note as committed and `report` is what
    /// validation said about it.
    Put {
        id: String,
        type_name: String,
        doc: Doc,
        report: NoteReport,
    },
    /// A note that was deleted.
    Delete { id: String },
}

/// A note naming an asset, from [`Index::asset_users`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetUse {
    pub note: String,
    pub type_name: String,
    pub title: String,
    pub archived: bool,
    /// Every way the note names the asset.
    pub how: Vec<AssetHow>,
}

/// One way a note names an asset. `via` is `field` for an asset field named `field`; otherwise
/// it is the kind of the block naming it (`image`, `asset`, or an old canvas `node`), inside
/// the content or canvas field `field`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetHow {
    pub field: String,
    pub via: String,
}

/// `index.db` and the tantivy index of one vault.
pub struct Index {
    conn: Connection,
    search: search::Search,
    assets: asset_text::AssetTexts,
}

impl Index {
    /// Open the index of `vault`: `.vault/index.db` and `.vault/search/`, creating either when it
    /// is missing. A new index is empty until [`Index::rebuild`] runs.
    pub fn open(vault: &store::Vault) -> Result<Index> {
        let dir = vault.server_dir();
        Index::open_at(&dir.join("index.db"), &dir.join("search"))
    }

    /// Open an index at explicit paths. A full-text index that is missing, or was written with
    /// another schema, is created empty, and the index is then behind whatever generation
    /// `index.db` recorded. PDF text is kept in `asset-text/v1/` beside `search_dir`.
    pub fn open_at(db: &Path, search_dir: &Path) -> Result<Index> {
        let conn = Connection::open(db)?;
        sql::init(&conn)?;
        sql::functions(&conn)?;
        let missing = !search_dir.join("meta.json").is_file();
        let (search, reset) = search::Search::open(search_dir)?;
        if missing || reset {
            sql::forget_generation(&conn)?;
        }
        let assets =
            asset_text::AssetTexts::open(&search_dir.with_file_name("asset-text").join("v1"))?;
        Ok(Index {
            conn,
            search,
            assets,
        })
    }

    /// Whether asset `id`'s text has been extracted, or found to have none.
    pub fn has_asset_text(&self, id: &str) -> bool {
        self.assets.has(id)
    }

    /// Keep what extracting asset `id`'s text came to. Notes indexed from now on take the text
    /// in; notes already indexed do so when next saved or rebuilt. [`Extracted::Busy`] keeps
    /// nothing, so a later rebuild tries again. A PDF pdf-extract could not read is logged in
    /// one line on standard error, once, since it is kept and not tried again.
    pub fn set_asset_text(&mut self, id: &str, extracted: &Extracted) -> Result<()> {
        match extracted {
            Extracted::Text(t) => self.assets.set(id, t),
            Extracted::Recovered { text, reason } => {
                eprintln!("asset {id}: PDF text read by the fallback extractor: {reason}");
                self.assets.set(id, text)
            }
            Extracted::Failed(reason) => {
                eprintln!("asset {id}: PDF text not extracted, kept as unreadable: {reason}");
                self.assets.set_unreadable(id, reason)
            }
            Extracted::Busy => Ok(()),
        }
    }

    /// Forget the text of asset `id`, which has been deleted.
    pub fn forget_asset_text(&mut self, id: &str) {
        self.assets.forget(id);
    }

    /// The vault generation the index holds every change up to, or `None` when it has never
    /// been rebuilt or its full-text half went missing.
    pub fn generation(&self) -> Result<Option<u64>> {
        sql::generation(&self.conn)
    }

    /// Whether the index holds every commit of `vault`, so a rebuild would change nothing.
    pub fn is_current(&self, vault: &store::Vault) -> Result<bool> {
        Ok(self.generation()? == Some(vault.generation()?))
    }

    /// Record that the commit that made `generation` has been applied: call it after the
    /// commit's [`Index::update`], or straight after a commit that changes no note. The index
    /// moves only from the generation before, so missing one commit leaves it behind.
    pub fn advance(&mut self, generation: u64) -> Result<()> {
        sql::advance_generation(&self.conn, generation)
    }

    /// The notes linked to or from note `id`, at most `limit` of them, most recently updated
    /// first, with every link among them and the note. Archived neighbours are left out, but an
    /// archived note has a graph of its own. `None` when the index does not hold `id`.
    pub fn graph(&self, id: &str, limit: usize) -> Result<Option<Graph>> {
        graph::graph(&self.conn, id, limit)
    }

    /// Every indexed note naming asset `asset`, with how it does, ordered by title then id.
    pub fn asset_users(&self, asset: &str) -> Result<Vec<AssetUse>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT r.note, n.type, n.title, n.archived, r.field, r.via
             FROM asset_refs r JOIN notes n ON n.id = r.note
             WHERE r.asset = ?1
             ORDER BY n.title COLLATE NOCASE, r.note, r.field, r.via",
        )?;
        let rows = stmt.query_map([asset], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                AssetHow {
                    field: r.get(4)?,
                    via: r.get(5)?,
                },
            ))
        })?;
        let mut out: Vec<AssetUse> = Vec::new();
        for row in rows {
            let (note, type_name, title, archived, how) = row?;
            match out.last_mut() {
                Some(last) if last.note == note => last.how.push(how),
                _ => out.push(AssetUse {
                    note,
                    type_name,
                    title,
                    archived,
                    how: vec![how],
                }),
            }
        }
        Ok(out)
    }

    /// How many indexed notes name each asset that any note names.
    pub fn asset_use_counts(&self) -> Result<std::collections::HashMap<String, usize>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT asset, COUNT(DISTINCT note) FROM asset_refs GROUP BY asset")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Whether the text extracted from asset `id` holds `lowercase_part`, which is lowercase.
    pub fn asset_text_matches(&self, id: &str, lowercase_part: &str) -> bool {
        self.assets.matches(id, lowercase_part)
    }

    /// The text extracted from asset `id`, as stored.
    pub fn asset_text(&self, id: &str) -> StoredText {
        self.assets.stored(id)
    }

    /// The SQLite connection, for `query` to read from.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Replace the taxonomy terms with those of `schema`. Call it after every schema change.
    pub fn set_schema(&mut self, schema: &Schema) -> Result<()> {
        let tx = self.conn.transaction()?;
        sql::replace_terms(&tx, schema)?;
        tx.commit()?;
        Ok(())
    }

    /// Apply one commit's note changes, all in one SQLite transaction and one tantivy commit.
    pub fn update(&mut self, schema: &Schema, updates: &[Update]) -> Result<()> {
        let mut docs = Vec::new();
        let tx = self.conn.transaction()?;
        for u in updates {
            match u {
                Update::Put {
                    id,
                    type_name,
                    doc,
                    report,
                } => {
                    let ty = schema
                        .type_def(type_name)
                        .ok_or_else(|| Error::UnknownType {
                            id: id.clone(),
                            type_name: type_name.clone(),
                        })?;
                    let mut note = project::project(ty, id, doc, report);
                    append_text(&mut note.search.body, &self.assets.of_doc(doc));
                    sql::delete_note(&tx, id)?;
                    sql::insert_note(&tx, &note)?;
                    docs.push(search::Change::Put(note.search));
                }
                Update::Delete { id } => {
                    sql::delete_note(&tx, id)?;
                    docs.push(search::Change::Delete(id.clone()));
                }
            }
        }
        self.search.apply(&docs)?;
        tx.commit()?;
        Ok(())
    }

    /// Rebuild both indexes from the vault files. Every file that fails validation is reported
    /// and left out, and every other file is indexed (D28). The index is then current with the
    /// vault's generation as it stood when the rebuild began.
    pub fn rebuild(&mut self, vault: &store::Vault) -> Result<RebuildReport> {
        rebuild::rebuild(self, vault)
    }

    /// Full-text search over titles, field values and content text (D16). Hits are ordered by
    /// score, then by id. `limit` of `None` returns every hit. Every word matches whole words
    /// except the last, which also matches as a prefix: `meet` finds `meeting`. A last word
    /// followed by a space, or of one character, matches whole words only. A prefix stands for
    /// at most the 50 most frequent words it starts in each of the title and the body.
    /// Archived notes are hits too: [`Index::search_page_where`] can leave them out.
    pub fn search(&self, text: &str, limit: Option<usize>) -> Result<Vec<Hit>> {
        self.search.search(text, limit)
    }

    /// Every hit of a full-text search among notes of `types`, or of every type when `None`, in
    /// no order. A hit's score is the one [`Index::search`] gives it. Archived notes are hits
    /// too.
    pub fn search_hits(&self, text: &str, types: Option<&[String]>) -> Result<Vec<Hit>> {
        self.search.hits(text, types)
    }

    /// One page of a full-text search: the best `size` hits strictly after `after`, the score
    /// and id of the previous page's last hit, ordered by score and then by id, and the count of
    /// every hit. Its cost grows with the page, not with the number of hits.
    pub fn search_page(
        &self,
        text: &str,
        after: Option<(f32, &str)>,
        size: usize,
    ) -> Result<SearchPage> {
        self.search.page(text, None, after, size)
    }

    /// [`Index::search_page`] among notes of `types` only.
    pub fn search_page_in(
        &self,
        text: &str,
        types: &[String],
        after: Option<(f32, &str)>,
        size: usize,
    ) -> Result<SearchPage> {
        self.search.page(text, Some(types), after, size)
    }

    /// [`Index::search_page`] among notes of `types`, or of every type when `None`, that are
    /// archived or not as `archived` says. Scores are the same whichever notes it reads.
    pub fn search_page_where(
        &self,
        text: &str,
        types: Option<&[String]>,
        archived: Archived,
        after: Option<(f32, &str)>,
        size: usize,
    ) -> Result<SearchPage> {
        self.search.page_where(text, types, archived, after, size)
    }

    /// The files the last rebuild could not index.
    pub fn failures(&self) -> Result<Vec<Failure>> {
        sql::failures(&self.conn)
    }

    /// Every row of `index.db`, one table after another, each sorted, as text. Two indexes of
    /// the same notes give the same dump.
    pub fn dump(&self) -> Result<String> {
        sql::dump(&self.conn)
    }
}

/// Add the text of a note's PDFs to its full-text body.
fn append_text(body: &mut String, extra: &str) {
    if extra.is_empty() {
        return;
    }
    if !body.is_empty() {
        body.push('\n');
    }
    body.push_str(extra);
}

fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}
