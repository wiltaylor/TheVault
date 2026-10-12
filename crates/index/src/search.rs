//! The tantivy full-text index in `.vault/search/` (D16).

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, STORED, STRING, Schema, TEXT, Value};
use tantivy::{IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

use crate::project::SearchDoc;
use crate::{Result, io_err};

/// Memory the writer may buffer before it flushes a segment.
const WRITER_HEAP: usize = 50_000_000;

/// One full-text match.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub score: f32,
}

pub(crate) enum Change {
    Put(SearchDoc),
    Delete(String),
}

pub(crate) struct Search {
    index: tantivy::Index,
    writer: IndexWriter,
    reader: IndexReader,
    id: Field,
    type_name: Field,
    title: Field,
    body: Field,
}

impl Search {
    pub fn open(dir: &Path) -> Result<Search> {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        let mut b = Schema::builder();
        let id = b.add_text_field("id", STRING | STORED);
        let type_name = b.add_text_field("type", STRING);
        let title = b.add_text_field("title", TEXT);
        let body = b.add_text_field("body", TEXT);
        let index = tantivy::Index::open_or_create(MmapDirectory::open(dir)?, b.build())?;
        let writer = index.writer_with_num_threads(1, WRITER_HEAP)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Search {
            index,
            writer,
            reader,
            id,
            type_name,
            title,
            body,
        })
    }

    /// Apply changes in one commit.
    pub fn apply(&mut self, changes: &[Change]) -> Result<()> {
        for c in changes {
            match c {
                Change::Put(d) => {
                    self.writer
                        .delete_term(Term::from_field_text(self.id, &d.id));
                    self.writer.add_document(doc!(
                        self.id => d.id.as_str(),
                        self.type_name => d.type_name.as_str(),
                        self.title => d.title.as_str(),
                        self.body => d.body.as_str(),
                    ))?;
                }
                Change::Delete(id) => {
                    self.writer.delete_term(Term::from_field_text(self.id, id));
                }
            }
        }
        self.commit()
    }

    /// Replace every document with `docs`, in one commit.
    pub fn replace_all(&mut self, docs: Vec<SearchDoc>) -> Result<()> {
        self.writer.delete_all_documents()?;
        let puts: Vec<Change> = docs.into_iter().map(Change::Put).collect();
        self.apply(&puts)
    }

    fn commit(&mut self) -> Result<()> {
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&self, text: &str, limit: Option<usize>) -> Result<Vec<Hit>> {
        let searcher = self.reader.searcher();
        let total = searcher.num_docs() as usize;
        let limit = limit.unwrap_or(total).min(total);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        parser.set_conjunction_by_default();
        let (query, _) = parser.parse_query_lenient(text);
        // Collect every match before cutting to `limit`, so ties break by id, not by segment.
        let found = searcher.search(&query, &TopDocs::with_limit(total).order_by_score())?;
        let mut hits = Vec::with_capacity(found.len());
        for (score, address) in found {
            let doc: TantivyDocument = searcher.doc(address)?;
            if let Some(id) = doc.get_first(self.id).and_then(|v| v.as_str()) {
                hits.push(Hit {
                    id: id.to_owned(),
                    score,
                });
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        hits.truncate(limit);
        Ok(hits)
    }
}
