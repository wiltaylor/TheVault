//! The inbox's one-time catch-up (TV-30): mark every email, message and call sent or made before
//! a date as processed, so the unified inbox does not start with years of old mail.
//!
//! Only `processed` changes, and only on notes where it is not set yet, so a second run changes
//! nothing. Fleeting notes are never touched. Nothing is sent anywhere: the Gmail sync never reads
//! `processed`. The query language has no datetime literal, so each type's unprocessed notes are
//! read through the index with their time field, and the date rule runs here. A note with no time,
//! or one that does not read as a datetime, is left alone.
//!
//! The writes go in chunks of [`CHUNK`] updates. Each chunk is one atomic batch with base versions,
//! and the write lock is free between chunks, so sync and other writes go in between. A note that
//! changes between the read and its chunk fails the version check: the chunk is read again and
//! retried once, and a note that is processed by then is skipped.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::json;
use state::Author;

use crate::batch::Op;
use crate::note::{Input, doc_to_json};
use crate::{Error, Result, Writer};

/// The types the catch-up covers, each with the field holding its own time.
pub const CATCH_UP_TYPES: [(&str, &str); 3] = [("email", "sent"), ("message", "sent"), ("call", "started")];

/// Updates per atomic batch.
pub const CHUNK: usize = 200;

const PROCESSED: &str = "processed";

/// What a catch-up matched and changed, per type in [`CATCH_UP_TYPES`] order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatchUp {
    pub dry_run: bool,
    /// `(type, notes before the date with processed not set)`.
    pub matched: Vec<(String, usize)>,
    /// How many notes the run set processed on: zero for a dry run.
    pub changed: usize,
    /// How many batches the run committed.
    pub chunks: usize,
}

impl CatchUp {
    pub fn total(&self) -> usize {
        self.matched.iter().map(|(_, n)| n).sum()
    }
}

/// `s` as an instant: an RFC 3339 datetime, or one with no offset, read as UTC.
pub fn instant(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .map(|t| t.and_utc())
}

impl Writer {
    /// Mark every email, message and call whose own time is before `before` processed, in chunks
    /// of [`CHUNK`]; with `dry_run`, only count them.
    pub fn mark_processed_before(&self, before: DateTime<Utc>, dry_run: bool, author: &Author) -> Result<CatchUp> {
        let mut out = CatchUp { dry_run, ..CatchUp::default() };
        let mut ids: Vec<String> = Vec::new();
        for (type_name, time) in CATCH_UP_TYPES {
            let found = self.unprocessed_before(type_name, time, before)?;
            out.matched.push((type_name.to_owned(), found.len()));
            ids.extend(found);
        }
        if dry_run {
            return Ok(out);
        }
        for chunk in ids.chunks(CHUNK) {
            let changed = match self.set_processed(chunk, author)? {
                Some(n) => n,
                // A note changed since it was read: read the chunk again and retry once.
                None => self.set_processed(chunk, author)?.ok_or_else(|| {
                    Error::Rejected("notes kept changing while they were marked processed; run it again".to_owned())
                })?,
            };
            if changed > 0 {
                out.changed += changed;
                out.chunks += 1;
            }
        }
        Ok(out)
    }

    /// The ids of the notes of `type_name` with `processed` not set whose `time` field is before
    /// `before`. A vault without the type, or whose type lacks `processed`, has none.
    fn unprocessed_before(&self, type_name: &str, time: &str, before: DateTime<Utc>) -> Result<Vec<String>> {
        let has_field = self
            .schema()
            .type_def(type_name)
            .is_some_and(|t| t.field(PROCESSED).is_some());
        if !has_field {
            return Ok(Vec::new());
        }
        let q = query::parse(&format!("from {type_name} where !{PROCESSED} select {time}"))
            .map_err(|e| Error::BadRequest(e.to_string()))?;
        let mut ids = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .read_index(|index, schema| {
                    query::run(
                        &q,
                        &query::Context::new(index, schema),
                        &query::PageRequest { cursor: cursor.take(), size: query::MAX_PAGE },
                    )
                })
                .map_err(|e| Error::BadRequest(e.to_string()))?;
            for row in page.rows {
                let at = row.fields.get(time).and_then(|v| v.as_str()).and_then(instant);
                if at.is_some_and(|t| t < before) {
                    ids.push(row.id);
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(ids),
            }
        }
    }

    /// Set `processed` on each note of `ids` that does not have it yet, as one batch. `None` when
    /// a note changed between its read and the write, so nothing was committed.
    fn set_processed(&self, ids: &[String], author: &Author) -> Result<Option<usize>> {
        let mut ops = Vec::new();
        for id in ids {
            let note = match self.read(id) {
                Ok(n) => n,
                // Deleted since it was found: nothing to mark.
                Err(Error::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if matches!(note.doc.field(PROCESSED), Some(schema::Value::Bool(true))) {
                continue;
            }
            let mut doc = doc_to_json(&note.doc);
            doc["fields"][PROCESSED] = json!(true);
            ops.push(Op::Update { id: id.clone(), base: Some(note.version), input: Input::Json(doc) });
        }
        if ops.is_empty() {
            return Ok(Some(0));
        }
        let n = ops.len();
        let report = self.batch(ops, false, author)?;
        if report.ok {
            return Ok(Some(n));
        }
        let conflict = report.items.iter().any(|i| matches!(i, Err(Error::Conflict { .. })));
        match (conflict, report.error) {
            (true, _) => Ok(None),
            (false, Some(e)) => Err(e),
            (false, None) => Err(report
                .items
                .into_iter()
                .find_map(|i| i.err())
                .unwrap_or_else(|| Error::Rejected("the batch failed".to_owned()))),
        }
    }
}
