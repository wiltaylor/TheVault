//! The tantivy full-text index in `.vault/search/` (D16).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::path::Path;

use tantivy::collector::{Collector, SegmentCollector};
use tantivy::columnar::{StrColumn, TermOrdHit};
use tantivy::directory::MmapDirectory;
use tantivy::query::{
    BooleanQuery, ConstScoreQuery, EmptyQuery, Occur, Query, QueryParser, TermQuery, TermSetQuery,
};
use tantivy::schema::{FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TEXT};
use tantivy::{
    DocId, IndexReader, IndexWriter, ReloadPolicy, Score, Searcher, SegmentOrdinal, SegmentReader,
    TantivyError, Term, doc,
};

use crate::project::SearchDoc;
use crate::{Archived, Result, io_err};

/// Memory the writer may buffer before it flushes a segment.
const WRITER_HEAP: usize = 50_000_000;

/// The fewest characters a last word needs to match as a prefix. A shorter one matches as a
/// whole word, since a one-letter prefix would match most of the index.
pub const MIN_PREFIX: usize = 2;

/// The most words of each field a prefix stands for: the most frequent, and the word itself
/// when it is one.
pub const MAX_EXPANSIONS: usize = 50;

/// Words the query syntax reads as operators, never as a prefix.
const OPERATORS: [&str; 3] = ["AND", "OR", "NOT"];

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
    /// `1` for an archived note, `0` for any other.
    archived: Field,
}

impl Search {
    /// Open or create the index in `dir`. The flag is true when an index written with another
    /// schema was found and replaced by an empty one.
    pub fn open(dir: &Path) -> Result<(Search, bool)> {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        let mut b = Schema::builder();
        // `id` is a fast field so a search can order ties by id without reading stored docs.
        let id = b.add_text_field("id", STRING | STORED | FAST);
        let type_name = b.add_text_field("type", STRING);
        let title = b.add_text_field("title", TEXT);
        let body = b.add_text_field("body", TEXT);
        let archived = b.add_u64_field("archived", INDEXED);
        let schema = b.build();
        let mut reset = false;
        let index = match tantivy::Index::open_or_create(MmapDirectory::open(dir)?, schema.clone())
        {
            // An index written with an older schema is derived data: start it again empty, and
            // let the caller know it must be rebuilt.
            Err(TantivyError::SchemaError(_)) => {
                std::fs::remove_dir_all(dir).map_err(io_err(dir))?;
                std::fs::create_dir_all(dir).map_err(io_err(dir))?;
                reset = true;
                tantivy::Index::create_in_dir(dir, schema)?
            }
            other => other?,
        };
        let writer = index.writer_with_num_threads(1, WRITER_HEAP)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let search = Search {
            index,
            writer,
            reader,
            id,
            type_name,
            title,
            body,
            archived,
        };
        Ok((search, reset))
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
                        self.archived => u64::from(d.archived),
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
        Ok(self.page(text, None, None, limit)?.hits)
    }

    /// Every match among notes of `types`, or of every type when `None`, in no order.
    pub fn hits(&self, text: &str, types: Option<&[String]>) -> Result<Vec<Hit>> {
        let searcher = self.reader.searcher();
        let query = self.query(&searcher, text, types)?;
        Ok(searcher.search(&query, &AllHits { id: "id" })?)
    }

    /// The best `size` matches strictly after `after`, a `(score, id)` from an earlier page,
    /// ordered by score and then by id, and the count of every match. Only notes of `types`
    /// match, or of every type when `None`. Only the page's hits are kept, and no stored
    /// document is read.
    pub fn page(
        &self,
        text: &str,
        types: Option<&[String]>,
        after: Option<(Score, &str)>,
        size: usize,
    ) -> Result<SearchPage> {
        self.page_where(text, types, Archived::Include, after, size)
    }

    /// [`Search::page`] among the notes `archived` names.
    pub fn page_where(
        &self,
        text: &str,
        types: Option<&[String]>,
        archived: Archived,
        after: Option<(Score, &str)>,
        size: usize,
    ) -> Result<SearchPage> {
        let searcher = self.reader.searcher();
        let query = self.query(&searcher, text, types)?;
        let flag = Term::from_field_u64(self.archived, 1);
        let flag = || Box::new(TermQuery::new(flag.clone(), IndexRecordOption::Basic));
        // Like a restriction to types, the archive state scores nothing.
        let query: Box<dyn Query> = match archived {
            Archived::Include => query,
            Archived::Exclude => Box::new(BooleanQuery::new(vec![
                (Occur::Must, query),
                (Occur::MustNot, flag() as Box<dyn Query>),
            ])),
            Archived::Only => Box::new(BooleanQuery::new(vec![
                (Occur::Must, query),
                (
                    Occur::Must,
                    Box::new(ConstScoreQuery::new(flag(), 0.0)) as Box<dyn Query>,
                ),
            ])),
        };
        let collector = TopHits {
            id: "id",
            size,
            after: after.map(|(s, id)| (s, id.to_owned())),
        };
        let (hits, total) = searcher.search(&query, &collector)?;
        Ok(SearchPage { hits, total })
    }

    /// The query `text` stands for. Words match whole words in the title or the body, and every
    /// word must match. The last word also matches as a prefix, for search as you type, unless
    /// the text ends in a space or the word is shorter than [`MIN_PREFIX`]; it then stands for
    /// the [`MAX_EXPANSIONS`] most frequent words it starts. A restriction to `types` scores
    /// nothing, so scores are the same with it or without it.
    fn query(
        &self,
        searcher: &Searcher,
        text: &str,
        types: Option<&[String]>,
    ) -> Result<Box<dyn Query>> {
        let mut parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        parser.set_conjunction_by_default();
        let words: Box<dyn Query> = match split_prefix(text) {
            None => parser.parse_query_lenient(text).0,
            Some((head, prefix)) => {
                let prefix = self.expand(searcher, &prefix)?;
                if head.trim().is_empty() {
                    prefix
                } else {
                    let (head, _) = parser.parse_query_lenient(head);
                    Box::new(BooleanQuery::intersection(vec![head, prefix]))
                }
            }
        };
        let Some(types) = types else {
            return Ok(words);
        };
        let terms = types
            .iter()
            .map(|t| Term::from_field_text(self.type_name, t));
        let only = ConstScoreQuery::new(Box::new(TermSetQuery::new(terms)), 0.0);
        Ok(Box::new(BooleanQuery::new(vec![
            (Occur::Must, words),
            (Occur::Must, Box::new(only)),
        ])))
    }

    /// A query for the words `prefix` starts, in the title or the body, scored as those words.
    fn expand(&self, searcher: &Searcher, prefix: &str) -> Result<Box<dyn Query>> {
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for field in [self.title, self.body] {
            // Each word's documents, summed over the segments.
            let mut words: HashMap<Vec<u8>, u64> = HashMap::new();
            for segment in searcher.segment_readers() {
                let inverted = segment.inverted_index(field)?;
                let mut stream = inverted
                    .terms()
                    .range()
                    .ge(prefix.as_bytes())
                    .into_stream()
                    .map_err(TantivyError::from)?;
                while stream.advance() {
                    if !stream.key().starts_with(prefix.as_bytes()) {
                        break;
                    }
                    let n = u64::from(stream.value().doc_freq);
                    *words.entry(stream.key().to_vec()).or_default() += n;
                }
            }
            let mut words: Vec<(Vec<u8>, u64)> = words.into_iter().collect();
            // The word itself first, then the most frequent, then in word order.
            words.sort_by(|(a, x), (b, y)| {
                let exact = |w: &[u8]| w != prefix.as_bytes();
                exact(a).cmp(&exact(b)).then(y.cmp(x)).then(a.cmp(b))
            });
            words.truncate(MAX_EXPANSIONS);
            for (word, _) in words {
                let Ok(word) = std::str::from_utf8(&word) else {
                    continue;
                };
                let term = Term::from_field_text(field, word);
                clauses.push((
                    Occur::Should,
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
                ));
            }
        }
        if clauses.is_empty() {
            return Ok(Box::new(EmptyQuery));
        }
        Ok(Box::new(BooleanQuery::new(clauses)))
    }
}

/// The text before the last word and the last word lowercased, when the last word matches as a
/// prefix: the text does not end in a space, and the word is letters and digits only, at least
/// [`MIN_PREFIX`] of them, outside a quoted phrase and not an operator.
fn split_prefix(text: &str) -> Option<(&str, String)> {
    if text.ends_with(char::is_whitespace) {
        return None;
    }
    let start = text
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let (head, last) = text.split_at(start);
    let usable = last.chars().all(char::is_alphanumeric)
        && last.chars().count() >= MIN_PREFIX
        && !OPERATORS.contains(&last)
        && head.matches('"').count() % 2 == 0;
    // The index drops words longer than 40 bytes, so a longer prefix could match nothing.
    let lower = last.to_lowercase();
    (usable && lower.len() <= 40).then_some((head, lower))
}

/// Collects every hit with its score, in no order.
struct AllHits {
    id: &'static str,
}

struct SegmentAll {
    ids: StrColumn,
    hits: Vec<(u64, Score)>,
}

impl Collector for AllHits {
    type Fruit = Vec<Hit>;
    type Child = SegmentAll;

    fn for_segment(
        &self,
        _: SegmentOrdinal,
        segment: &SegmentReader,
    ) -> tantivy::Result<SegmentAll> {
        let ids = segment.fast_fields().str(self.id)?.ok_or_else(|| {
            TantivyError::SchemaError(format!("`{}` is not a fast field", self.id))
        })?;
        Ok(SegmentAll {
            ids,
            hits: Vec::new(),
        })
    }

    fn requires_scoring(&self) -> bool {
        true
    }

    fn merge_fruits(&self, fruits: Vec<Vec<Hit>>) -> tantivy::Result<Vec<Hit>> {
        Ok(fruits.into_iter().flatten().collect())
    }
}

impl SegmentCollector for SegmentAll {
    type Fruit = Vec<Hit>;

    fn collect(&mut self, doc: DocId, score: Score) {
        if let Some(ord) = self.ids.ords().first(doc) {
            self.hits.push((ord, score));
        }
    }

    fn harvest(mut self) -> Vec<Hit> {
        // Ids in ordinal order read the dictionary once, block by block, rather than seeking
        // for every hit.
        self.hits.sort_unstable_by_key(|(ord, _)| *ord);
        let mut out = Vec::with_capacity(self.hits.len());
        let mut scores = self.hits.iter().map(|(_, s)| *s);
        let ords = self.hits.iter().map(|(o, _)| *o);
        let _ = self.ids.dictionary().sorted_ords_to_term_cb(ords, |bytes| {
            if let (Ok(id), Some(score)) = (std::str::from_utf8(bytes), scores.next()) {
                out.push(Hit {
                    id: id.to_owned(),
                    score,
                });
            }
            Ok(())
        });
        out
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
