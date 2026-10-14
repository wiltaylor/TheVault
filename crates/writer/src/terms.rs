//! Open taxonomies: a note saved with a term an open taxonomy does not hold yet adds the term
//! (`allow_new = true`, docs/vault-format.md "Taxonomy files").
//!
//! The added terms are written in the same commit as the notes that hold them, so a write that
//! fails leaves neither the note nor the term behind.

use std::collections::BTreeMap;

use schema::{Doc, FieldKind, Invalid, Issue, Schema, Taxonomy, TypeDef, Value};
use store::Change;

use crate::defs::taxonomy_path;
use crate::{Error, NoteChange, Result, note_path};

/// How a term path a note holds is matched to an open taxonomy's terms.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Match {
    /// A name that matches a term's without case is that term, and the note is rewritten to
    /// hold its exact path. For writes, so `rust` picks an existing `Rust`.
    Resolve,
    /// Names match only exactly, and the note is never rewritten. For a planned type change,
    /// whose notes are kept as they are.
    Exact,
}

/// The taxonomies a write grows, as they will be.
#[derive(Default)]
pub(crate) struct Growth {
    taxonomies: BTreeMap<String, Taxonomy>,
    /// How many terms have been added so far.
    pub(crate) added: usize,
}

impl Growth {
    /// Add to the open taxonomies every term `doc`, a note of `ty`, holds that they lack.
    /// `schema` gives the taxonomies as they are before this write. Problems are returned as
    /// issues, such as a term whose parent does not exist.
    pub(crate) fn note(
        &mut self,
        schema: &Schema,
        ty: &TypeDef,
        doc: &mut Doc,
        how: Match,
    ) -> Vec<Issue> {
        let mut issues = Vec::new();
        for (name, value) in doc.fields.iter_mut() {
            let Some(f) = ty.field(name) else { continue };
            let FieldKind::Taxonomy { taxonomy } = &f.kind else {
                continue;
            };
            let Some(base) = schema.taxonomy(taxonomy).filter(|t| t.allow_new) else {
                continue;
            };
            let tax = self
                .taxonomies
                .entry(taxonomy.clone())
                .or_insert_with(|| base.clone());
            let mut terms: Vec<&mut String> = match value {
                Value::List(l) => l
                    .iter_mut()
                    .filter_map(|v| match v {
                        Value::Str(s) => Some(s),
                        _ => None,
                    })
                    .collect(),
                Value::Str(s) => vec![s],
                _ => Vec::new(),
            };
            // Parents first, so a write may add a term and a term under it together.
            terms.sort_by_key(|s| s.matches('/').count());
            for term in terms {
                if term.is_empty() || tax.contains(term) {
                    continue;
                }
                match tax.resolve_term(term, how == Match::Exact, true) {
                    Ok((path, new)) => {
                        self.added += usize::from(new);
                        if how == Match::Resolve {
                            *term = path;
                        }
                    }
                    Err(message) => issues.push(Issue::new(name.clone(), message)),
                }
            }
        }
        issues
    }

    /// Run [`Growth::note`] over every put in `changes`, failing on the first note with a
    /// problem.
    pub(crate) fn changes(schema: &Schema, changes: &mut [NoteChange]) -> Result<Growth> {
        let mut growth = Growth::default();
        for c in changes.iter_mut() {
            let (Some(doc), Some(ty)) = (c.doc.as_mut(), schema.type_def(&c.type_name)) else {
                continue;
            };
            let issues = growth.note(schema, ty, doc, Match::Resolve);
            if !issues.is_empty() {
                return Err(Error::Invalid(Invalid {
                    file: note_path(&c.type_name, &c.id),
                    issues,
                }));
            }
        }
        Ok(growth)
    }

    /// The taxonomies grown so far, as they are now.
    pub(crate) fn taxonomies(&self) -> impl Iterator<Item = &Taxonomy> {
        self.taxonomies.values()
    }

    /// `schema` with the grown taxonomies in it and their files, or `None` when nothing grew.
    pub(crate) fn finish(self, schema: &Schema) -> Result<Option<(Schema, Vec<Change>)>> {
        let grown: Vec<Taxonomy> = self
            .taxonomies
            .into_values()
            .filter(|t| schema.taxonomy(&t.name) != Some(t))
            .collect();
        if grown.is_empty() {
            return Ok(None);
        }
        let mut schema = schema.clone();
        let mut files = Vec::with_capacity(grown.len());
        for t in grown {
            let path = taxonomy_path(&t.name);
            let text = store::canonical(&t.to_wcl(), &path)?;
            // What is written must read back as the taxonomy it was made from.
            let back = Taxonomy::parse(&t.name, &text)
                .map_err(|e| Error::InvalidType(schema::SchemaError(vec![e])))?;
            debug_assert_eq!(back, t);
            files.push(Change::write(path, text));
            schema.grow_taxonomy(back);
        }
        Ok(Some((schema, files)))
    }
}
