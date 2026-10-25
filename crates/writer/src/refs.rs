//! Step 6 of the write path: the reverse sides of two-way and peer references (D4) and the
//! `on_delete` rules (D5).
//!
//! The submitted changes are the starting state. Each live note's reference fields are compared
//! with what they held when the note was last looked at, and every difference is mirrored onto
//! the other notes:
//!
//! - `two_way`: gaining B puts this note into B's inverse field, and losing B takes it out. A
//!   single inverse field that pointed elsewhere is overwritten, and that change is mirrored in
//!   turn, so the displaced note loses B. A field that is its own inverse, such as `siblings` on
//!   `person`, mirrors into the same field of B; B's own mirroring then finds this note already
//!   there. Unlike `peer`, it is pairwise: B and C on this note's list are not put on each
//!   other's. A note cannot list itself in such a field: [`lists_itself`] refuses it.
//! - `peer`: gaining B makes this note's group, itself and its list, a complete group: every
//!   member lists every other. Losing B takes B out of the group: B drops every member and every
//!   member drops B.
//!
//! A mirrored edit can cause further edits, so notes are compared until none has changed. Every
//! edit is idempotent, which is what makes that loop end.
//!
//! A delete looks up every note whose reference field points at the deleted note and applies that
//! field's `on_delete`: `unlink` drops the reference, `cascade` deletes the referrer as well, and
//! `restrict` refuses the whole write, listing every live referrer that restricts.

use std::collections::{HashMap, VecDeque};

use schema::{Doc, FieldDef, Issue, OnDelete, Relation, Schema, Value};

use crate::{Error, Event, Inner, Input, NoteChange, Result, Writer, note, note_path, time};

/// A note that refused a delete through a `restrict` reference field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referrer {
    pub id: String,
    pub type_name: String,
    /// The field on the referrer that points at the deleted note.
    pub field: String,
}

/// One note this write may touch.
struct Entry {
    type_name: String,
    event: Event,
    /// Whether the caller submitted this change. A submitted change is written even if step 6
    /// leaves it as it was.
    submitted: bool,
    /// The note as it is on disk, `None` for a note this write creates.
    disk: Option<Doc>,
    /// The note as it was when its references were last mirrored.
    seen: Option<Doc>,
    /// The note as it will be written, `None` once it is deleted.
    doc: Option<Doc>,
}

struct Work<'a> {
    writer: &'a Writer,
    inner: &'a Inner,
    order: Vec<String>,
    notes: HashMap<String, Entry>,
    /// Notes whose references may differ from `seen`.
    dirty: VecDeque<String>,
    /// Notes deleted whose referrers are not yet handled.
    deletes: VecDeque<String>,
}

/// The submitted changes with every change they imply added, submitted changes first.
pub(crate) fn expand(
    writer: &Writer,
    inner: &Inner,
    changes: Vec<NoteChange>,
) -> Result<Vec<NoteChange>> {
    let mut work = Work {
        writer,
        inner,
        order: Vec::new(),
        notes: HashMap::new(),
        dirty: VecDeque::new(),
        deletes: VecDeque::new(),
    };
    for c in changes {
        let disk = match c.event {
            Event::Create => None,
            Event::Update | Event::Delete => work.load(&c.id, &c.type_name)?,
        };
        match c.event {
            Event::Delete => work.deletes.push_back(c.id.clone()),
            _ => work.dirty.push_back(c.id.clone()),
        }
        work.order.push(c.id.clone());
        work.notes.insert(
            c.id,
            Entry {
                type_name: c.type_name,
                event: c.event,
                submitted: true,
                seen: disk.clone(),
                disk,
                doc: c.doc,
            },
        );
    }

    let mut restricted = Vec::new();
    loop {
        if let Some(id) = work.dirty.pop_front() {
            work.mirror(&id)?;
        } else if let Some(id) = work.deletes.pop_front() {
            restricted.extend(work.on_delete(&id)?);
        } else {
            break;
        }
    }

    // A restricting referrer that this write deletes as well does not hold the delete back.
    restricted.retain(|(_, r): &(String, Referrer)| work.is_live(&r.id));
    if let Some((id, _)) = restricted.first() {
        let id = id.clone();
        let referrers = restricted
            .into_iter()
            .filter(|(target, _)| *target == id)
            .map(|(_, r)| r)
            .collect();
        return Err(Error::Restricted { id, referrers });
    }

    work.finish()
}

impl Work<'_> {
    /// The note on disk, parsed, or `None` when there is no such file.
    fn load(&self, id: &str, type_name: &str) -> Result<Option<Doc>> {
        let path = note_path(type_name, id);
        if !self.writer.vault.root().join(&path).is_file() {
            return Ok(None);
        }
        let text = self.writer.vault.read_text(&path)?;
        Ok(Some(Input::Wcl(text).parse(&path)?))
    }

    fn is_live(&self, id: &str) -> bool {
        match self.notes.get(id) {
            Some(e) => e.doc.is_some(),
            None => true,
        }
    }

    /// The entry for a live note, loading it from disk on first use. `None` when the note does
    /// not exist, is deleted, or is not of `want_type`. Validation reports a reference to such a
    /// note, so it is left alone here.
    fn entry(&mut self, id: &str, want_type: Option<&str>) -> Result<Option<&mut Entry>> {
        if !self.notes.contains_key(id) {
            let Some(type_name) = self.writer.locate(id)? else {
                return Ok(None);
            };
            let Some(disk) = self.load(id, &type_name)? else {
                return Ok(None);
            };
            self.order.push(id.to_owned());
            self.notes.insert(
                id.to_owned(),
                Entry {
                    type_name,
                    event: Event::Update,
                    submitted: false,
                    seen: Some(disk.clone()),
                    doc: Some(disk.clone()),
                    disk: Some(disk),
                },
            );
        }
        let entry = self.notes.get_mut(id).expect("inserted above");
        if entry.doc.is_none() || want_type.is_some_and(|t| t != entry.type_name) {
            return Ok(None);
        }
        Ok(Some(entry))
    }

    /// Change field `field` of live note `id` of type `type_name` with `edit`, queueing the note
    /// when its references changed.
    fn edit(
        &mut self,
        id: &str,
        type_name: &str,
        field: &str,
        edit: impl FnOnce(&FieldDef, &mut Doc),
    ) -> Result<()> {
        let Some(def) = self
            .inner
            .schema
            .type_def(type_name)
            .and_then(|t| t.field(field))
            .cloned()
        else {
            return Ok(());
        };
        let Some(entry) = self.entry(id, Some(type_name))? else {
            return Ok(());
        };
        let doc = entry.doc.as_mut().expect("entry returns live notes");
        let before = ids(doc, field);
        edit(&def, doc);
        if ids(doc, field) != before {
            self.dirty.push_back(id.to_owned());
        }
        Ok(())
    }

    /// Mirror every two-way and peer difference between note `id`'s `seen` and current doc.
    fn mirror(&mut self, id: &str) -> Result<()> {
        let Some(entry) = self.notes.get_mut(id) else {
            return Ok(());
        };
        let Some(doc) = entry.doc.clone() else {
            return Ok(());
        };
        let seen = entry.seen.replace(doc.clone());
        let type_name = entry.type_name.clone();
        let Some(ty) = self.inner.schema.type_def(&type_name) else {
            return Ok(());
        };
        for field in ty.fields.clone() {
            let Some(r) = field.reference() else {
                continue;
            };
            let old = seen
                .as_ref()
                .map(|d| ids(d, &field.name))
                .unwrap_or_default();
            let new = ids(&doc, &field.name);
            let gained: Vec<String> = new.iter().filter(|b| !old.contains(b)).cloned().collect();
            let lost: Vec<String> = old.iter().filter(|b| !new.contains(b)).cloned().collect();
            match r.relation {
                Relation::OneWay => {}
                Relation::TwoWay => {
                    let (Some(target), Some(inverse)) = (r.target.clone(), r.inverse.clone())
                    else {
                        continue;
                    };
                    for b in gained.iter().filter(|b| *b != id) {
                        self.edit(b, &target, &inverse, |def, d| add(d, def, id))?;
                    }
                    for b in lost.iter().filter(|b| *b != id) {
                        self.edit(b, &target, &inverse, |def, d| remove(d, def, id))?;
                    }
                }
                Relation::Peer => {
                    let name = &field.name;
                    for b in lost.iter().filter(|b| *b != id) {
                        let group: Vec<String> = std::iter::once(id.to_owned())
                            .chain(new.iter().cloned())
                            .collect();
                        self.edit(b, &type_name, name, |def, d| {
                            for m in &group {
                                remove(d, def, m);
                            }
                        })?;
                        for m in new.iter().filter(|m| *m != id) {
                            self.edit(m, &type_name, name, |def, d| remove(d, def, b))?;
                        }
                    }
                    if !gained.is_empty() {
                        let group: Vec<String> = std::iter::once(id.to_owned())
                            .chain(new.iter().filter(|m| *m != id).cloned())
                            .collect();
                        for m in &group[1..] {
                            self.edit(m, &type_name, name, |def, d| {
                                for other in group.iter().filter(|o| *o != m) {
                                    add(d, def, other);
                                }
                            })?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Apply `on_delete` for every note that points at deleted note `id`. Returns the referrers
    /// that restrict.
    fn on_delete(&mut self, id: &str) -> Result<Vec<(String, Referrer)>> {
        let mut candidates = self.indexed_referrers(id)?;
        for other in &self.order {
            if !candidates.contains(other) {
                candidates.push(other.clone());
            }
        }
        let mut restricted = Vec::new();
        for a in candidates.iter().filter(|a| *a != id) {
            let Some(entry) = self.entry(a, None)? else {
                continue;
            };
            let type_name = entry.type_name.clone();
            let doc = entry.doc.clone().expect("entry returns live notes");
            let Some(ty) = self.inner.schema.type_def(&type_name) else {
                continue;
            };
            for field in &ty.fields {
                let Some(r) = field.reference() else {
                    continue;
                };
                if !ids(&doc, &field.name).iter().any(|t| t == id) {
                    continue;
                }
                match r.on_delete {
                    OnDelete::Unlink => {
                        self.edit(a, &type_name, &field.name, |def, d| remove(d, def, id))?;
                    }
                    OnDelete::Restrict => restricted.push((
                        id.to_owned(),
                        Referrer {
                            id: a.clone(),
                            type_name: type_name.clone(),
                            field: field.name.clone(),
                        },
                    )),
                    OnDelete::Cascade => {
                        let entry = self.notes.get_mut(a).expect("loaded above");
                        if entry.doc.take().is_some() {
                            entry.event = Event::Delete;
                            self.deletes.push_back(a.clone());
                        }
                    }
                }
            }
            // A reference in a dict's entry follows its own `on_delete` too (types T5).
            for field in &ty.fields {
                let schema::FieldKind::Dict { keys, values } = &field.kind else {
                    continue;
                };
                let entries: Vec<(String, Value)> = doc
                    .blocks
                    .iter()
                    .find(|b| b.kind == "dict" && b.label() == Some(field.name.as_str()))
                    .map(|b| b.body.fields.clone())
                    .unwrap_or_default();
                for (key, value) in entries {
                    let def = keys.iter().find(|k| k.name == key).or(match values {
                        schema::DictValues::Of(d) => Some(d.as_ref()),
                        _ => None,
                    });
                    let Some(r) = def.and_then(FieldDef::reference) else {
                        continue;
                    };
                    let holds = match &value {
                        Value::Str(s) => s == id,
                        Value::List(l) => l.iter().any(|v| v.as_str() == Some(id)),
                        _ => false,
                    };
                    if !holds {
                        continue;
                    }
                    match r.on_delete {
                        OnDelete::Unlink => {
                            let entry = self.notes.get_mut(a).expect("loaded above");
                            let Some(doc) = entry.doc.as_mut() else {
                                continue;
                            };
                            if let Some(b) = doc.blocks.iter_mut().find(|b| {
                                b.kind == "dict" && b.label() == Some(field.name.as_str())
                            }) {
                                b.body.fields.retain_mut(|(k, v)| {
                                    if k != &key {
                                        return true;
                                    }
                                    match v {
                                        Value::List(l) => {
                                            l.retain(|x| x.as_str() != Some(id));
                                            true
                                        }
                                        _ => false,
                                    }
                                });
                            }
                        }
                        OnDelete::Restrict => restricted.push((
                            id.to_owned(),
                            Referrer {
                                id: a.clone(),
                                type_name: type_name.clone(),
                                field: format!("{}.{key}", field.name),
                            },
                        )),
                        OnDelete::Cascade => {
                            let entry = self.notes.get_mut(a).expect("loaded above");
                            if entry.doc.take().is_some() {
                                entry.event = Event::Delete;
                                self.deletes.push_back(a.clone());
                            }
                        }
                    }
                }
            }
        }
        Ok(restricted)
    }

    /// The notes whose reference fields point at `id`, from `index.db`.
    fn indexed_referrers(&self, id: &str) -> Result<Vec<String>> {
        let conn = self.inner.index.connection();
        let mut stmt = conn
            .prepare("SELECT DISTINCT src FROM links WHERE dst = ?1 AND via = 'field' ORDER BY src")
            .map_err(index::Error::from)?;
        let rows = stmt
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(index::Error::from)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(index::Error::from)?);
        }
        Ok(out)
    }

    /// The changes to commit: every submitted change, then every other note that changed.
    fn finish(mut self) -> Result<Vec<NoteChange>> {
        let now = time::utc(self.writer.clock.now_ms());
        let mut out = Vec::new();
        for id in &self.order {
            let entry = self
                .notes
                .remove(id)
                .expect("every ordered id has an entry");
            if !entry.submitted && entry.doc == entry.disk {
                continue;
            }
            let doc = match entry.doc {
                Some(doc) => {
                    let ty = self
                        .inner
                        .schema
                        .type_def(&entry.type_name)
                        .ok_or_else(|| Error::UnknownType(entry.type_name.clone()))?;
                    let created = crate::created_of(&doc).unwrap_or_else(|| now.clone());
                    Some(note::arrange(ty, doc, &created, &now))
                }
                None => None,
            };
            out.push(NoteChange {
                id: id.clone(),
                type_name: entry.type_name,
                event: if doc.is_none() {
                    Event::Delete
                } else {
                    entry.event
                },
                doc,
            });
        }
        Ok(out)
    }
}

/// An issue for each field of note `id` that is its own inverse and holds the note itself: a
/// note is not its own sibling or its own spouse.
pub(crate) fn lists_itself(schema: &Schema, type_name: &str, id: &str, doc: &Doc) -> Vec<Issue> {
    let Some(ty) = schema.type_def(type_name) else {
        return Vec::new();
    };
    ty.fields
        .iter()
        .filter(|f| {
            f.reference().is_some_and(|r| {
                r.relation == Relation::TwoWay
                    && r.target.as_deref() == Some(type_name)
                    && r.inverse.as_deref() == Some(f.name.as_str())
            })
        })
        .filter(|f| ids(doc, &f.name).iter().any(|x| x == id))
        .map(|f| {
            Issue::new(
                f.name.clone(),
                "cannot hold the note itself: the field is its own inverse",
            )
        })
        .collect()
}

/// The note ids a reference field holds: one for a single field, every string for a list.
fn ids(doc: &Doc, field: &str) -> Vec<String> {
    match doc.field(field) {
        Some(Value::Str(s)) => vec![s.clone()],
        Some(Value::List(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// Put `id` into reference field `def`: append it to a list, or replace a single value.
fn add(doc: &mut Doc, def: &FieldDef, id: &str) {
    let value = if def.multiple {
        let mut list = ids(doc, &def.name);
        if list.iter().any(|x| x == id) {
            return;
        }
        list.push(id.to_owned());
        Value::List(list.into_iter().map(Value::Str).collect())
    } else {
        Value::Str(id.to_owned())
    };
    set(doc, &def.name, Some(value));
}

/// Take `id` out of reference field `def`. A field left empty is removed, as an optional field
/// with no value is left out of a note file.
fn remove(doc: &mut Doc, def: &FieldDef, id: &str) {
    let list = ids(doc, &def.name);
    if !list.iter().any(|x| x == id) {
        return;
    }
    let rest: Vec<String> = list.into_iter().filter(|x| x != id).collect();
    let value = match (def.multiple, rest.is_empty()) {
        (_, true) => None,
        (true, false) => Some(Value::List(rest.into_iter().map(Value::Str).collect())),
        (false, false) => Some(Value::Str(rest[0].clone())),
    };
    set(doc, &def.name, value);
}

/// Replace, add or remove field `name`. A new field goes at the end; `arrange` orders it.
fn set(doc: &mut Doc, name: &str, value: Option<Value>) {
    let at = doc.fields.iter().position(|(n, _)| n == name);
    match (at, value) {
        (Some(i), Some(v)) => doc.fields[i].1 = v,
        (Some(i), None) => {
            doc.fields.remove(i);
        }
        (None, Some(v)) => doc.fields.push((name.to_owned(), v)),
        (None, None) => {}
    }
}
