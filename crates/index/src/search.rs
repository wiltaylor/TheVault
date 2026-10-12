//! The tantivy full-text index in `.vault/search/` (D16).

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::Path;

use tantivy::collector::{Collector, SegmentCollector};
use tantivy::columnar::{StrColumn, TermOrdHit};
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{FAST, Field, STORED, STRING, Schema, TEXT};
use tantivy::{
    DocId, IndexReader, IndexWriter, ReloadPolicy, Score, SegmentOrdinal, SegmentReader,
    TantivyError, Term, doc,
};

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

/// One page of full-text matches.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchPage {
    /// The best matches after the cursor, best first.
    pub hits: Vec<Hit>,
    /// Every match, whatever the cursor and page size.
    pub total: usize,
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
        // `id` is a fast field so a search can order ties by id without reading stored docs.
        let id = b.add_text_field("id", STRING | STORED | FAST);
        let type_name = b.add_text_field("type", STRING);
        let title = b.add_text_field("title", TEXT);
        let body = b.add_text_field("body", TEXT);
        let schema = b.build();
        let index = match tantivy::Index::open_or_create(MmapDirectory::open(dir)?, schema.clone())
        {
            // An index written with an older schema is derived data: start it again empty, and
            // the rebuild that follows every open fills it.
            Err(TantivyError::SchemaError(_)) => {
                std::fs::remove_dir_all(dir).map_err(io_err(dir))?;
                std::fs::create_dir_all(dir).map_err(io_err(dir))?;
                tantivy::Index::create_in_dir(dir, schema)?
            }
            other => other?,
        };
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

    /// Every match, or the best `limit`, ordered by score and then by id.
    pub fn search(&self, text: &str, limit: Option<usize>) -> Result<Vec<Hit>> {
        let limit = limit.unwrap_or(usize::MAX);
        Ok(self.page(text, None, limit)?.hits)
    }

    /// The best `size` matches strictly after `after`, a `(score, id)` from an earlier page,
    /// ordered by score and then by id, and the count of every match. Only the page's hits are
    /// kept, and no stored document is read.
    pub fn page(
        &self,
        text: &str,
        after: Option<(Score, &str)>,
        size: usize,
    ) -> Result<SearchPage> {
        let searcher = self.reader.searcher();
        let mut parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        parser.set_conjunction_by_default();
        let (query, _) = parser.parse_query_lenient(text);
        let collector = TopHits {
            id: "id",
            size,
            after: after.map(|(s, id)| (s, id.to_owned())),
        };
        let (hits, total) = searcher.search(&query, &collector)?;
        Ok(SearchPage { hits, total })
    }
}

/// Keeps the best `size` hits by score then id, after an optional cursor, and counts every hit.
struct TopHits {
    id: &'static str,
    size: usize,
    after: Option<(Score, String)>,
}

/// A segment's hit, ordered so the worst is greatest: the heap's top is the one to evict.
struct Candidate {
    score: Score,
    ord: u64,
}

impl Candidate {
    fn worse(&self, other: &Candidate) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.ord.cmp(&other.ord))
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.worse(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.worse(other)
    }
}

struct SegmentHits {
    ids: StrColumn,
    size: usize,
    /// The cursor in this segment's terms: a hit with exactly this score is after the cursor
    /// when its id's ordinal is at least this.
    after: Option<(Score, u64)>,
    heap: BinaryHeap<Candidate>,
    count: usize,
}

impl SegmentHits {
    fn ord(&self, doc: DocId) -> u64 {
        self.ids.ords().first(doc).unwrap_or(u64::MAX)
    }
}

impl Collector for TopHits {
    type Fruit = (Vec<Hit>, usize);
    type Child = SegmentHits;

    fn for_segment(
        &self,
        _: SegmentOrdinal,
        segment: &SegmentReader,
    ) -> tantivy::Result<SegmentHits> {
        let ids = segment.fast_fields().str(self.id)?.ok_or_else(|| {
            TantivyError::SchemaError(format!("`{}` is not a fast field", self.id))
        })?;
        let after = match &self.after {
            Some((score, id)) => {
                let first = match ids.dictionary().term_ord_or_next(id)? {
                    TermOrdHit::Exact(ord) => ord.saturating_add(1),
                    TermOrdHit::Next(ord) => ord,
                };
                Some((*score, first))
            }
            None => None,
        };
        Ok(SegmentHits {
            ids,
            size: self.size,
            after,
            heap: BinaryHeap::with_capacity(self.size.min(1024)),
            count: 0,
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(&self, fruits: Vec<(Vec<Hit>, usize)>) -> tantivy::Result<(Vec<Hit>, usize)> {
        let mut total = 0;
        let mut hits = Vec::new();
        for (h, n) in fruits {
            hits.extend(h);
            total += n;
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        hits.truncate(self.size);
        Ok((hits, total))
    }
}

impl SegmentCollector for SegmentHits {
    type Fruit = (Vec<Hit>, usize);

    fn collect(&mut self, doc: DocId, score: Score) {
        self.count += 1;
        if self.size == 0 {
            return;
        }
        let mut ord = None;
        if let Some((after, first)) = self.after {
            match score.total_cmp(&after) {
                Ordering::Greater => return,
                Ordering::Equal => {
                    let o = self.ord(doc);
                    if o < first {
                        return;
                    }
                    ord = Some(o);
                }
                Ordering::Less => {}
            }
        }
        if self.heap.len() == self.size
            && let Some(worst) = self.heap.peek()
        {
            if score < worst.score {
                return;
            }
            let o = ord.unwrap_or_else(|| self.ord(doc));
            let c = Candidate { score, ord: o };
            if c >= *worst {
                return;
            }
            self.heap.pop();
            self.heap.push(c);
            return;
        }
        let o = ord.unwrap_or_else(|| self.ord(doc));
        self.heap.push(Candidate { score, ord: o });
    }

    fn harvest(self) -> (Vec<Hit>, usize) {
        let mut hits = Vec::with_capacity(self.heap.len());
        let mut id = String::new();
        for c in self.heap {
            id.clear();
            if matches!(self.ids.ord_to_str(c.ord, &mut id), Ok(true)) {
                hits.push(Hit {
                    id: id.clone(),
                    score: c.score,
                });
            }
        }
        (hits, self.count)
    }
}
