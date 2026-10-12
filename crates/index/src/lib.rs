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
//! | `notes` | Note: id, type, path, title, created, updated, both again as Unix time, flagged, and its fields as JSON |
//! | `field_values` | Scalar inside a field value, including `title`, `created` and `updated`, with the type of its note |
//! | `links` | Note reference in a field, or mention in inline text, from `src` to `dst` |
//! | `terms` | Taxonomy term, with its parent, depth and display position |
//! | `term_ancestors` | Pair of a term and itself or a term above it, for `under()` |
//! | `failures` | Vault file the last rebuild could not index, with its diagnostic |
//! | `meta` | Fact about the index itself: `generation` |

mod project;
mod rebuild;
mod search;
mod sql;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use schema::{Doc, NoteReport, Schema};

pub use project::mentions;
pub use rebuild::{Failure, RebuildReport};
pub use search::{Hit, SearchPage};

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

/// `index.db` and the tantivy index of one vault.
pub struct Index {
    conn: Connection,
    search: search::Search,
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
    /// `index.db` recorded.
    pub fn open_at(db: &Path, search_dir: &Path) -> Result<Index> {
        let conn = Connection::open(db)?;
        sql::init(&conn)?;
        let missing = !search_dir.join("meta.json").is_file();
        let (search, reset) = search::Search::open(search_dir)?;
        if missing || reset {
            sql::forget_generation(&conn)?;
        }
        Ok(Index { conn, search })
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
                    let note = project::project(ty, id, doc, report);
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
    /// score, then by id. `limit` of `None` returns every hit.
    pub fn search(&self, text: &str, limit: Option<usize>) -> Result<Vec<Hit>> {
        self.search.search(text, limit)
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
        self.search.page(text, after, size)
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

fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}
