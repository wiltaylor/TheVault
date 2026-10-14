//! Batches of note writes, and upserts by a key field (D32).
//!
//! A batch is one write. Every operation is prepared against the vault as it stands, then all of
//! them go through steps 5 to 11 of the write path together and land in one atomic commit, or none
//! of them does. The report carries a result for every item either way.
//!
//! A dry run takes the same path and stops just before the commit, so it reports what the real
//! run would do: the same outcome for every item, the same errors, the same side effects and the
//! text every file would hold. Only the ids of new notes and the version tokens differ, because
//! those are made when the real run happens.
//!
//! No item depends on another: two items may not change the same note, and two upserts may not
//! share a key value. A batch that breaks this rule fails on the later item.

use std::collections::HashMap;

use schema::{Doc, FieldKind, Value};
use state::Author;

use crate::{
    Error, Event, Inner, Input, NoteChange, Result, Saved, Writer, note, note_path, set_field,
};

/// One operation in a batch.
#[derive(Debug, Clone)]
pub enum Op {
    /// Create a note of `type_name`.
    Create { type_name: String, input: Input },
    /// Replace note `id`, which must be at version `base`.
    Update {
        id: String,
        input: Input,
        base: Option<String>,
    },
    /// Delete note `id`, which must be at version `base`.
    Delete { id: String, base: Option<String> },
    /// Find the note of `type_name` whose field `key` holds the submitted note's value for it.
    /// Update that note with the submitted fields and blocks, keeping the rest, or create one when
    /// none matches. `base`, when given, must be the matched note's version.
    Upsert {
        type_name: String,
        key: String,
        input: Input,
        base: Option<String>,
    },
}

/// What an item did, or would do in a dry run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
    Deleted,
    /// An upsert matched a note that already holds everything submitted. Nothing is written.
    Unchanged,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Created => "created",
            Outcome::Updated => "updated",
            Outcome::Deleted => "deleted",
            Outcome::Unchanged => "unchanged",
        }
    }
}

/// The result of one item that did not fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub outcome: Outcome,
    pub id: String,
    pub type_name: String,
    /// The note's version after the batch. `None` in a dry run, and for every item of a batch
    /// that did not commit.
    pub version: Option<String>,
    /// The canonical WCL the note holds after the batch, or for a delete the WCL that went to the
    /// trash. `None` when the batch failed before the text was worked out.
    pub wcl: Option<String>,
}

/// What a batch did, or would do.
#[derive(Debug)]
pub struct Report {
    pub dry_run: bool,
    /// Every item succeeded. A real run committed exactly when this is true.
    pub ok: bool,
    /// One result per operation, in order.
    pub items: Vec<std::result::Result<Item, Error>>,
    /// Other notes the batch changed: reverse references and `on_delete` effects (D4, D5).
    pub also: Vec<Saved>,
    /// A failure no single item caused, such as a handler rejecting the batch.
    pub error: Option<Error>,
}

/// An operation ready to apply.
struct Prepared {
    outcome: Outcome,
    id: String,
    type_name: String,
    /// The note change, `None` for an unchanged upsert.
    change: Option<NoteChange>,
    /// For an unchanged upsert: the note's version and text.
    current: Option<(String, String)>,
    /// What the item changes, for the duplicate check: the note id, and an upsert's key value.
    targets: Vec<String>,
}

/// The field kinds an upsert key may have: single values that compare as one string or integer.
fn is_key_kind(kind: &FieldKind) -> bool {
    matches!(
        kind,
        FieldKind::String
            | FieldKind::Int
            | FieldKind::Url
            | FieldKind::Date
            | FieldKind::Datetime
            | FieldKind::Select { .. }
    )
}

impl Writer {
    /// Run `ops` as one write, or as a dry run of one.
    pub fn batch(&self, ops: Vec<Op>, dry_run: bool, author: &Author) -> Result<Report> {
        let _w = self.write_lock();
        let inner = self.lock();
        let mut prepared: Vec<Result<Prepared>> = ops
            .into_iter()
            .map(|op| {
                let mut p = self.prepare_op(&inner, op, author)?;
                // Validate each item on its own first, so every invalid item is reported, not
                // only the first the combined write meets.
                if let Some(change) = &mut p.change {
                    let one = std::slice::from_mut(change);
                    match crate::terms::Growth::changes(&inner.schema, one)?
                        .finish(&inner.schema)?
                    {
                        Some((grown, _)) => self.render(&grown, one)?,
                        None => self.render(&inner.schema, one)?,
                    };
                }
                Ok(p)
            })
            .collect();

        let mut seen: HashMap<String, usize> = HashMap::new();
        for (n, p) in prepared.iter_mut().enumerate() {
            let Ok(item) = p else { continue };
            let clash = item
                .targets
                .iter()
                .find_map(|t| seen.get(t).map(|m| (t, *m)));
            if let Some((target, m)) = clash {
                *p = Err(Error::BadRequest(format!(
                    "item {n} changes the same note as item {m} ({})",
                    target.replace('\0', " ")
                )));
                continue;
            }
            for t in &item.targets {
                seen.insert(t.clone(), n);
            }
        }

        let unapplied = |p: &Prepared| Item {
            outcome: p.outcome,
            id: p.id.clone(),
            type_name: p.type_name.clone(),
            version: None,
            wcl: None,
        };
        if prepared.iter().any(Result::is_err) {
            return Ok(Report {
                dry_run,
                ok: false,
                items: prepared
                    .into_iter()
                    .map(|p| p.map(|p| unapplied(&p)))
                    .collect(),
                also: Vec::new(),
                error: None,
            });
        }
        let prepared: Vec<Prepared> = prepared.into_iter().map(|p| p.expect("checked")).collect();

        let changes: Vec<NoteChange> = prepared.iter().filter_map(|p| p.change.clone()).collect();
        let result = if changes.is_empty() {
            Ok(Vec::new())
        } else {
            self.apply_with(inner, changes, &|| Ok(()), author, Vec::new(), dry_run)
        };

        match result {
            Ok(saved) => {
                let mut saved = saved.into_iter();
                let mut items = Vec::with_capacity(prepared.len());
                for p in &prepared {
                    let item = if p.change.is_some() {
                        let s = saved
                            .next()
                            .expect("a saved note for every submitted change");
                        Item {
                            outcome: p.outcome,
                            id: s.id,
                            type_name: s.type_name,
                            version: (!dry_run).then_some(s.version),
                            wcl: Some(s.wcl),
                        }
                    } else {
                        let (version, wcl) = p.current.clone().unwrap_or_default();
                        Item {
                            outcome: p.outcome,
                            id: p.id.clone(),
                            type_name: p.type_name.clone(),
                            version: Some(version),
                            wcl: Some(wcl),
                        }
                    };
                    items.push(Ok(item));
                }
                let also = saved
                    .map(|mut s| {
                        if dry_run && s.event != Event::Delete {
                            s.version = String::new();
                        }
                        s
                    })
                    .collect();
                Ok(Report {
                    dry_run,
                    ok: true,
                    items,
                    also,
                    error: None,
                })
            }
            Err(e) => {
                let culprit = match &e {
                    Error::Invalid(inv) => prepared
                        .iter()
                        .position(|p| note_path(&p.type_name, &p.id) == inv.file),
                    Error::Restricted { id, .. } => prepared.iter().position(|p| &p.id == id),
                    _ => None,
                };
                let mut items: Vec<std::result::Result<Item, Error>> =
                    prepared.iter().map(|p| Ok(unapplied(p))).collect();
                let error = match culprit {
                    Some(n) => {
                        items[n] = Err(e);
                        None
                    }
                    None => Some(e),
                };
                Ok(Report {
                    dry_run,
                    ok: false,
                    items,
                    also: Vec::new(),
                    error,
                })
            }
        }
    }

    /// Run one upsert as a batch of one.
    pub fn upsert(
        &self,
        type_name: &str,
        key: &str,
        input: &Input,
        base: Option<&str>,
        dry_run: bool,
        author: &Author,
    ) -> Result<Report> {
        let op = Op::Upsert {
            type_name: type_name.to_owned(),
            key: key.to_owned(),
            input: input.clone(),
            base: base.map(str::to_owned),
        };
        self.batch(vec![op], dry_run, author)
    }

    fn prepare_op(&self, inner: &Inner, op: Op, author: &Author) -> Result<Prepared> {
        match op {
            Op::Create { type_name, input } => {
                self.prepare_create(inner, &type_name, &input, author, Vec::new())
            }
            Op::Update { id, input, base } => {
                let (type_name, old) = self.existing(&id)?;
                let doc = input.parse(&note_path(&type_name, &id))?;
                let change = self.prepare_put(inner, &id, &type_name, doc, Some(&old), author)?;
                self.check_base(&id, &type_name, &old, base.as_deref())?;
                Ok(Prepared {
                    outcome: Outcome::Updated,
                    targets: vec![id.clone()],
                    id,
                    type_name,
                    change: Some(change),
                    current: None,
                })
            }
            Op::Delete { id, base } => {
                let (type_name, old) = self.existing(&id)?;
                self.check_base(&id, &type_name, &old, base.as_deref())?;
                Ok(Prepared {
                    outcome: Outcome::Deleted,
                    targets: vec![id.clone()],
                    change: Some(NoteChange {
                        id: id.clone(),
                        type_name: type_name.clone(),
                        event: Event::Delete,
                        doc: None,
                    }),
                    id,
                    type_name,
                    current: None,
                })
            }
            Op::Upsert {
                type_name,
                key,
                input,
                base,
            } => self.prepare_upsert(inner, &type_name, &key, &input, base.as_deref(), author),
        }
    }

    fn prepare_create(
        &self,
        inner: &Inner,
        type_name: &str,
        input: &Input,
        author: &Author,
        targets: Vec<String>,
    ) -> Result<Prepared> {
        let id = ulid::Ulid::new().to_string();
        let doc = input.parse(&note_path(type_name, &id))?;
        let change = self.prepare_put(inner, &id, type_name, doc, None, author)?;
        Ok(Prepared {
            outcome: Outcome::Created,
            id,
            type_name: type_name.to_owned(),
            change: Some(change),
            current: None,
            targets,
        })
    }

    fn prepare_upsert(
        &self,
        inner: &Inner,
        type_name: &str,
        key: &str,
        input: &Input,
        base: Option<&str>,
        author: &Author,
    ) -> Result<Prepared> {
        let ty = inner
            .schema
            .type_def(type_name)
            .ok_or_else(|| Error::UnknownType(type_name.to_owned()))?;
        if key != "title" {
            match ty.field(key) {
                None => {
                    return Err(Error::BadRequest(format!(
                        "type {type_name:?} has no field {key:?} to upsert by"
                    )));
                }
                Some(f) if f.multiple || !is_key_kind(&f.kind) => {
                    return Err(Error::BadRequest(format!(
                        "field {key:?} cannot be an upsert key: a key is `title` or a single \
                         string, int, url, date, datetime or select field"
                    )));
                }
                Some(_) => {}
            }
        }
        let doc = input.parse(&note_path(type_name, "upsert"))?;
        let value = match doc.field(key) {
            Some(v @ (Value::Str(_) | Value::Int(_))) => v.clone(),
            Some(_) => {
                return Err(Error::BadRequest(format!(
                    "the key field {key:?} must hold a string or an integer"
                )));
            }
            None => {
                return Err(Error::BadRequest(format!(
                    "the note has no value for its key field {key:?}"
                )));
            }
        };
        let key_target = format!(
            "{type_name}\0{key}\0{}",
            match &value {
                Value::Str(s) => s.clone(),
                other => format!("{other:?}"),
            }
        );

        let matches = self.find_by_key(inner, type_name, key, &value)?;
        let id = match matches.as_slice() {
            [] => return self.prepare_create(inner, type_name, input, author, vec![key_target]),
            [id] => id.clone(),
            _ => {
                return Err(Error::KeyNotUnique {
                    type_name: type_name.to_owned(),
                    key: key.to_owned(),
                    ids: matches,
                });
            }
        };

        let (_, old) = self.existing(&id)?;
        if base.is_some() {
            self.check_base(&id, type_name, &old, base)?;
        }
        let path = note_path(type_name, &id);
        let old_doc = Input::Wcl(old.clone()).parse(&path)?;
        let merged = merge(old_doc.clone(), doc);
        let change = self.prepare_put(inner, &id, type_name, merged, Some(&old), author)?;

        // The update is a no-op when, stamped with the note's own `updated`, it prints as the
        // note already is.
        let unchanged = change.doc.as_ref().is_some_and(|d| {
            let mut d = d.clone();
            if let Some(updated) = old_doc.field("updated").and_then(Value::as_str) {
                set_field(&mut d, "updated", updated.to_owned());
            }
            store::canonical(&note::print(&d), &path).is_ok_and(|text| text == old)
        });
        let targets = vec![id.clone(), key_target];
        if unchanged {
            let version = self.head(&id, type_name, &old)?;
            return Ok(Prepared {
                outcome: Outcome::Unchanged,
                id,
                type_name: type_name.to_owned(),
                change: None,
                current: Some((version, old)),
                targets,
            });
        }
        Ok(Prepared {
            outcome: Outcome::Updated,
            id,
            type_name: type_name.to_owned(),
            change: Some(change),
            current: None,
            targets,
        })
    }

    /// The ids of the notes of `type_name` whose field `key` holds `value`, from the index.
    fn find_by_key(
        &self,
        inner: &Inner,
        type_name: &str,
        key: &str,
        value: &Value,
    ) -> Result<Vec<String>> {
        let conn = inner.index.connection();
        let collect = |sql: &str, params: &[&str]| -> Result<Vec<String>> {
            let mut stmt = conn.prepare(sql).map_err(index::Error::from)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                    r.get::<_, String>(0)
                })
                .map_err(index::Error::from)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(index::Error::from)?);
            }
            Ok(out)
        };
        match (key, value) {
            ("title", Value::Str(s)) => collect(
                "SELECT id FROM notes WHERE type = ?1 AND title = ?2 ORDER BY id",
                &[type_name, s],
            ),
            (_, Value::Str(s)) => collect(
                "SELECT DISTINCT v.note FROM field_values v JOIN notes n ON n.id = v.note
                 WHERE n.type = ?1 AND v.field = ?2 AND v.pos = 0 AND v.part = '' AND v.str = ?3
                 ORDER BY v.note",
                &[type_name, key, s],
            ),
            (_, Value::Int(i)) => collect(
                "SELECT DISTINCT v.note FROM field_values v JOIN notes n ON n.id = v.note
                 WHERE n.type = ?1 AND v.field = ?2 AND v.pos = 0 AND v.part = ''
                 AND v.num = CAST(?3 AS INTEGER) ORDER BY v.note",
                &[type_name, key, &i.to_string()],
            ),
            _ => Ok(Vec::new()),
        }
    }
}

/// `old` with every field and block of `new` set on it. A block replaces the block of the same
/// kind and labels.
fn merge(mut old: Doc, new: Doc) -> Doc {
    for (name, value) in new.fields {
        match old.fields.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => old.fields.push((name, value)),
        }
    }
    for block in new.blocks {
        match old
            .blocks
            .iter_mut()
            .find(|b| b.kind == block.kind && b.labels == block.labels)
        {
            Some(slot) => *slot = block,
            None => old.blocks.push(block),
        }
    }
    old
}
