//! Planned type changes, type delete, and the flagged-note listing (D6).
//!
//! [`Writer::put_type`] creates a type, or replaces one that has no notes. A change to a type that
//! has notes is two calls. [`Writer::plan_type_change`] works out every note
//! the change edits and returns a [`TypePlan`] without touching a file. [`Writer::apply_type_change`]
//! takes the same change with the plan's token, plans it again, and commits the type file and
//! every edited note as one commit only when the new plan's token matches. The token covers the
//! old and new type text, the renames, and the id and version of every note of the type, so a
//! note changed or created since planning fails the apply with [`Error::PlanStale`] rather than
//! being overwritten.
//!
//! A change can name a wscript [`Migration`] for what the simple rules cannot express. It runs
//! once over every note of the type, after the simple rules, in both the plan and the apply, and
//! what it returns joins the same commit (D6). A migration that fails on any note fails the whole
//! change with [`Error::Migration`], before anything is written. The token covers what the
//! migration returned, so a script that changed since planning makes the plan stale.
//!
//! A schema change runs no handlers: it records a version of every note it edits, authored by the
//! caller.

use std::collections::BTreeMap;

use index::Update;
use schema::{
    Doc, FieldChanges, Invalid, Issue, NoteEdit, NoteReport, Schema, SchemaError, TypeDef, Value,
};
use sha2::{Digest, Sha256};
use state::Author;
use store::Change;

use crate::terms::{Growth, Match};
use crate::{
    Error, Event, Inner, Input, Lookup, Result, Saved, Writer, note, note_path, time, type_path,
};

/// A change to an existing type, as the client submitted it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeChange {
    /// The new text of `types/<type>.wcl`.
    pub wcl: String,
    /// Old field name to new field name.
    pub renames: BTreeMap<String, String>,
    /// A script to run over every note of the type.
    pub migration: Option<Migration>,
}

/// A wscript migration: function `function` of `scripts/<script>.ws` (D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub script: String,
    pub function: String,
}

/// What a type change would do, from [`Writer::plan_type_change`].
#[derive(Debug, Clone, PartialEq)]
pub struct TypePlan {
    pub type_name: String,
    /// Pass this back to [`Writer::apply_type_change`] to apply exactly this plan.
    pub token: String,
    /// The new type file in canonical form.
    pub wcl: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub renamed: Vec<(String, String)>,
    /// Every note whose file the change edits, in id order.
    pub changes: Vec<PlannedNote>,
    /// Every note of the type that holds data the new type does not describe, in id order.
    pub flagged: Vec<String>,
}

/// One note a plan edits.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedNote {
    pub id: String,
    pub title: String,
    /// The note's version when the plan was made.
    pub version: String,
    pub edits: Vec<NoteEdit>,
}

/// A note that holds items its type does not describe (D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flagged {
    pub id: String,
    pub type_name: String,
    pub title: String,
    pub version: String,
    /// The items the type does not describe.
    pub unknown: Vec<String>,
}

/// A planned change with everything applying it needs.
struct Worked {
    plan: TypePlan,
    schema: Schema,
    /// The files of the open taxonomies the change adds terms to.
    taxonomies: Vec<Change>,
    /// Every note of the type: its id, the doc after the change, and whether the change edits it.
    notes: Vec<(String, Doc, bool)>,
}

impl Writer {
    /// Plan a change to type `name` without applying it.
    pub fn plan_type_change(&self, name: &str, change: &TypeChange) -> Result<TypePlan> {
        let _w = self.write_lock();
        let current = self.schema();
        Ok(self.work(&current, name, change)?.plan)
    }

    /// Apply a change to type `name` that [`Writer::plan_type_change`] planned with `token`.
    /// The type file and every edited note change in one commit (D4, D6).
    pub fn apply_type_change(
        &self,
        name: &str,
        change: &TypeChange,
        token: &str,
        author: &Author,
    ) -> Result<(TypePlan, Vec<Saved>)> {
        let _w = self.write_lock();
        let current = self.schema();
        let worked = self.work(&current, name, change)?;
        if worked.plan.token != token {
            return Err(Error::PlanStale(Box::new(worked.plan)));
        }
        let Worked {
            plan,
            schema,
            taxonomies,
            notes,
        } = worked;
        let ty = schema
            .type_def(name)
            .expect("the planned schema holds the type");
        let now = time::utc(self.clock.now_ms());
        let lookup = Lookup {
            writer: self,
            pending: Default::default(),
            deleted: Default::default(),
        };

        let mut files = vec![Change::write(type_path(name), plan.wcl.as_bytes())];
        files.extend(taxonomies);
        let mut staged = Vec::with_capacity(notes.len());
        for (id, doc, edited) in notes {
            let path = note_path(name, &id);
            let (doc, text) = if edited {
                let created = crate::created_of(&doc).unwrap_or_else(|| now.clone());
                let doc = note::arrange(ty, doc, &created, &now);
                let text = store::canonical(&note::print(&doc), &path)?;
                let doc = Input::Wcl(text.clone()).parse(&path)?;
                files.push(Change::write(path.clone(), text.as_bytes()));
                (doc, Some(text))
            } else {
                (doc, None)
            };
            let report = schema
                .validate_note(name, &doc, &lookup)
                .unwrap_or_else(|_| NoteReport::default());
            staged.push((id, doc, report, text));
        }
        let generation = self.commit(files)?;

        let mut saved = Vec::new();
        let mut updates = Vec::with_capacity(staged.len());
        for (id, doc, report, text) in staged {
            if let Some(text) = text {
                let v = self.state.record_version(&id, name, author, &text)?;
                saved.push(Saved {
                    id: id.clone(),
                    type_name: name.to_owned(),
                    event: Event::Update,
                    version: v.token,
                    wcl: text,
                    also: Vec::new(),
                });
            }
            updates.push(Update::Put {
                id,
                type_name: name.to_owned(),
                doc,
                report,
            });
        }
        let mut inner = self.lock();
        let inner = &mut *inner;
        install(inner, schema)?;
        inner.index.update(&inner.schema, &updates)?;
        inner.index.advance(generation)?;
        self.announce(&saved, &|_| None);
        Ok((plan, saved))
    }

    /// Delete type `name`. A type that still has notes is not deleted.
    pub fn delete_type(&self, name: &str) -> Result<()> {
        if crate::is_built_in_type(name) {
            return Err(Error::BuiltInType(name.to_owned()));
        }
        let _w = self.write_lock();
        let mut inner = self.lock();
        if inner.schema.type_def(name).is_none() {
            return Err(Error::UnknownType(name.to_owned()));
        }
        let notes = self.note_ids(name)?;
        if !notes.is_empty() {
            return Err(Error::TypeHasNotes {
                type_name: name.to_owned(),
                notes,
            });
        }
        let types = inner
            .schema
            .types()
            .filter(|t| t.name != name)
            .cloned()
            .collect();
        let schema = rebuild_schema(&inner.schema, types)?;
        let generation = self.commit(vec![Change::delete(type_path(name))])?;
        install(&mut inner, schema)?;
        inner.index.advance(generation)?;
        Ok(())
    }

    /// Every note that holds items its type does not describe, in id order (D6).
    pub fn flagged(&self) -> Result<Vec<Flagged>> {
        let inner = self.lock();
        let rows: Vec<(String, String, String)> = {
            let conn = inner.index.connection();
            let mut stmt = conn
                .prepare("SELECT id, type, title FROM notes WHERE flagged ORDER BY id")
                .map_err(index::Error::from)?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(index::Error::from)?
                .collect::<std::result::Result<_, _>>()
                .map_err(index::Error::from)?
        };
        let lookup = Lookup {
            writer: self,
            pending: Default::default(),
            deleted: Default::default(),
        };
        let mut out = Vec::with_capacity(rows.len());
        for (id, type_name, title) in rows {
            let path = note_path(&type_name, &id);
            let text = self.vault.read_text(&path)?;
            let doc = Input::Wcl(text.clone()).parse(&path)?;
            let unknown = inner
                .schema
                .validate_note(&type_name, &doc, &lookup)
                .map(|r| r.unknown)
                .unwrap_or_default();
            out.push(Flagged {
                version: self.head(&id, &type_name, &text)?,
                id,
                type_name,
                title,
                unknown,
            });
        }
        Ok(out)
    }

    /// Plan a change against the `current` schema: check it, work out every note's edits, run the
    /// migration, and validate the result. Runs without the data lock, so a migration can read
    /// the vault; the caller holds the write lock.
    fn work(&self, current: &Schema, name: &str, change: &TypeChange) -> Result<Worked> {
        let old = current
            .type_def(name)
            .ok_or_else(|| Error::UnknownType(name.to_owned()))?;
        let old_text = self.vault.read_text(&type_path(name))?;
        let (new, text) = parse_type(name, &change.wcl)?;
        let rules = FieldChanges::new(old, &new, &change.renames)
            .map_err(|e| Error::InvalidType(SchemaError(vec![e])))?;
        let types = current
            .types()
            .map(|t| {
                if t.name == name {
                    new.clone()
                } else {
                    t.clone()
                }
            })
            .collect();
        let mut schema = rebuild_schema(current, types)?;
        crate::builtin::check_built_in_types(&schema)?;
        // A term a note holds that an open taxonomy lacks is added with the change, so a list of
        // strings can become a list of terms of an open taxonomy.
        let base = schema.clone();
        let new_ty = base
            .type_def(name)
            .expect("the rebuilt schema holds the type");
        let mut growth = Growth::default();

        let lookup = Lookup {
            writer: self,
            pending: Default::default(),
            deleted: Default::default(),
        };
        let mut hasher = Sha256::new();
        for part in [name, &old_text, &text] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        for (from, to) in &change.renames {
            hasher.update(format!("{from}\0{to}\0").as_bytes());
        }
        if let Some(m) = &change.migration {
            hasher.update(format!("migration\0{}\0{}\0", m.script, m.function).as_bytes());
        }

        let mut changes = Vec::new();
        let mut flagged = Vec::new();
        let mut invalid = Vec::new();
        let mut notes = Vec::new();
        for id in self.note_ids(name)? {
            let path = note_path(name, &id);
            let text = self.vault.read_text(&path)?;
            let version = self.head(&id, name, &text)?;
            hasher.update(format!("{id}\0{version}\0").as_bytes());
            let doc = Input::Wcl(text).parse(&path)?;
            let (mut after, mut edits) = rules.apply(&doc);
            if let Some(m) = &change.migration {
                let migrated = self
                    .hooks
                    .migrate(name, m, &id, &after)
                    .map_err(|message| Error::Migration {
                        id: id.clone(),
                        message,
                    })?;
                if let Some(migrated) = migrated.map(|d| keep_stamps(&after, d))
                    && migrated != after
                {
                    edits.push(NoteEdit::Migrated {
                        script: m.script.clone(),
                        function: m.function.clone(),
                    });
                    after = migrated;
                }
                hasher.update(note::print(&after).as_bytes());
                hasher.update([0]);
            }
            let added = growth.added;
            // A term that cannot be added stays invalid, and validation says so.
            growth.note(&base, new_ty, &mut after, Match::Exact);
            if growth.added != added {
                for t in growth.taxonomies() {
                    schema.grow_taxonomy(t.clone());
                }
            }
            match schema.validate_note(name, &after, &lookup) {
                Ok(report) => {
                    if report.is_flagged() {
                        flagged.push(id.clone());
                    }
                }
                // A note that was already invalid does not hold the change up.
                Err(issues) if current.validate_note(name, &doc, &lookup).is_ok() => {
                    invalid.push(Invalid {
                        file: path.clone(),
                        issues,
                    });
                }
                Err(_) => {}
            }
            if !edits.is_empty() {
                changes.push(PlannedNote {
                    id: id.clone(),
                    title: doc
                        .field("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    version,
                    edits: edits.clone(),
                });
            }
            notes.push((id, after, !edits.is_empty()));
        }
        if !invalid.is_empty() {
            return Err(Error::InvalidType(SchemaError(invalid)));
        }
        let token = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let taxonomies = growth
            .finish(&base)?
            .map(|(_, files)| files)
            .unwrap_or_default();
        Ok(Worked {
            taxonomies,
            plan: TypePlan {
                type_name: name.to_owned(),
                token,
                wcl: text,
                added: rules.added,
                removed: rules.removed,
                renamed: rules.renamed,
                changes,
                flagged,
            },
            schema,
            notes,
        })
    }

    /// The ids of every note of type `name` on disk, sorted.
    fn note_ids(&self, name: &str) -> Result<Vec<String>> {
        let dir = self.vault.root().join("notes").join(name);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(Error::Store(store::Error::Io { path: dir, source })),
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let file = e.file_name().to_string_lossy().into_owned();
                let id = file.strip_suffix(".wcl")?;
                schema::is_ulid(id).then(|| id.to_owned())
            })
            .collect();
        ids.sort();
        Ok(ids)
    }
}

/// Check a submitted type file: a valid name, data-only, and a valid definition on its own.
/// Returns the definition and the file's canonical text.
fn parse_type(name: &str, wcl: &str) -> Result<(TypeDef, String)> {
    if !schema::is_name(name) {
        return Err(Error::InvalidType(SchemaError(vec![Invalid {
            file: type_path(name),
            issues: vec![Issue::new(
                "file",
                format!("{name:?} is not a valid type name"),
            )],
        }])));
    }
    let text = store::canonical(wcl, &type_path(name))?;
    let ty = TypeDef::parse(name, &text).map_err(|e| Error::InvalidType(SchemaError(vec![e])))?;
    Ok((ty, text))
}

/// The schema with `types` in place of its types, checked across types and taxonomies.
fn rebuild_schema(current: &Schema, types: Vec<TypeDef>) -> Result<Schema> {
    Schema::new(types, current.taxonomies().cloned().collect()).map_err(Error::InvalidType)
}

/// `migrated` with the `created` and `updated` times and the archived flag of `before` where the
/// script left them out.
fn keep_stamps(before: &Doc, mut migrated: Doc) -> Doc {
    for stamp in ["created", "updated", note::ARCHIVED, note::CONFLICT_OF] {
        if migrated.field(stamp).is_none()
            && let Some(v) = before.field(stamp)
        {
            migrated.fields.push((stamp.to_owned(), v.clone()));
        }
    }
    migrated
}

/// Make `schema` the writer's schema.
fn install(inner: &mut Inner, schema: Schema) -> Result<()> {
    inner.index.set_schema(&schema)?;
    inner.schema = schema;
    Ok(())
}
