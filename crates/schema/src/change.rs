//! Schema-change planning: what changing a type does to the notes of that type (D6).
//!
//! [`FieldChanges::new`] compares the old and new definitions of one type, with the renames the
//! caller named, and refuses a change the simple rules cannot carry out. [`FieldChanges::apply`]
//! then works out the edits to one note:
//!
//! - A renamed field's value moves to the new name. A compound field's block is relabelled.
//! - A required field the note lacks receives the field's default. A newly added required field
//!   must carry one; a newly added optional field writes nothing.
//! - A removed field's data stays in the note, which validation then flags.

use std::collections::{BTreeMap, HashSet};

use crate::doc::{Doc, Value};
use crate::types::TypeDef;
use crate::{Invalid, Issue};

/// One edit a schema change makes to one note.
#[derive(Debug, Clone, PartialEq)]
pub enum NoteEdit {
    /// The value of field `from` moved to field `to`.
    Rename { from: String, to: String },
    /// Field `field` received the default `value`.
    Default { field: String, value: Value },
    /// Function `function` of migration script `script` changed the note.
    Migrated { script: String, function: String },
}

/// The field-level rules of one type change.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChanges {
    /// Fields of the new type that are neither in the old one nor a rename target.
    pub added: Vec<String>,
    /// Fields of the old type that are neither in the new one nor renamed.
    pub removed: Vec<String>,
    /// Renames as `(from, to)`, sorted by `from`.
    pub renamed: Vec<(String, String)>,
    /// Required, non-compound fields with a default, as `(name, default)`, in type-file order.
    defaults: Vec<(String, Value)>,
    /// Old and new block kind of each renamed compound field, by old name.
    compound: BTreeMap<String, String>,
}

impl FieldChanges {
    /// Check a change from `old` to `new` of the type `new.name`. `renames` maps old field names
    /// to new ones. Every problem is reported against the type file.
    pub fn new(
        old: &TypeDef,
        new: &TypeDef,
        renames: &BTreeMap<String, String>,
    ) -> Result<FieldChanges, Invalid> {
        let mut issues = Vec::new();
        let targets: HashSet<&str> = renames.values().map(String::as_str).collect();
        let mut seen_targets = HashSet::new();
        let mut compound = BTreeMap::new();
        for (from, to) in renames {
            let at = format!("renames.{from}");
            let old_field = old.field(from);
            let new_field = new.field(to);
            if old_field.is_none() {
                issues.push(Issue::new(
                    &at,
                    format!("the type has no field {from:?} to rename"),
                ));
            }
            if new_field.is_none() {
                issues.push(Issue::new(
                    &at,
                    format!("the new definition has no field {to:?}"),
                ));
            }
            if from == to {
                issues.push(Issue::new(&at, "renames a field to itself"));
            }
            if !seen_targets.insert(to.as_str()) {
                issues.push(Issue::new(
                    &at,
                    format!("{to:?} is the target of more than one rename"),
                ));
            }
            if new.field(from).is_some() && !targets.contains(from.as_str()) {
                issues.push(Issue::new(
                    &at,
                    format!("{from:?} is renamed, so the new definition cannot keep it"),
                ));
            }
            if old.field(to).is_some() && !renames.contains_key(to) {
                issues.push(Issue::new(
                    &at,
                    format!("{to:?} already holds data; rename or remove it first"),
                ));
            }
            if let (Some(o), Some(n)) = (old_field, new_field) {
                match (o.kind.is_compound(), n.kind.is_compound()) {
                    (true, true) => {
                        compound.insert(from.clone(), n.kind.name().to_owned());
                    }
                    (false, false) => {}
                    _ => issues.push(Issue::new(
                        &at,
                        format!(
                            "cannot move a value of kind {} into a field of kind {}; use a migration",
                            o.kind.name(),
                            n.kind.name()
                        ),
                    )),
                }
            }
        }

        let added: Vec<String> = new
            .fields
            .iter()
            .filter(|f| old.field(&f.name).is_none() && !targets.contains(f.name.as_str()))
            .map(|f| f.name.clone())
            .collect();
        for name in &added {
            let f = new.field(name).expect("an added field is in the new type");
            if f.required && f.default.is_none() {
                issues.push(Issue::new(
                    format!("field {name}.default"),
                    "a new required field needs a default, which every existing note receives",
                ));
            }
        }
        let removed = old
            .fields
            .iter()
            .filter(|f| new.field(&f.name).is_none() && !renames.contains_key(&f.name))
            .map(|f| f.name.clone())
            .collect();

        if !issues.is_empty() {
            return Err(Invalid {
                file: format!("types/{}.wcl", new.name),
                issues,
            });
        }
        Ok(FieldChanges {
            added,
            removed,
            renamed: renames
                .iter()
                .map(|(f, t)| (f.clone(), t.clone()))
                .collect(),
            defaults: new
                .fields
                .iter()
                .filter(|f| f.required && !f.kind.is_compound())
                .filter_map(|f| Some((f.name.clone(), f.default.clone()?)))
                .collect(),
            compound,
        })
    }

    /// The note after the change, and the edits that got it there. No edits means the note's
    /// file stays as it is.
    pub fn apply(&self, doc: &Doc) -> (Doc, Vec<NoteEdit>) {
        let mut doc = doc.clone();
        let mut edits = Vec::new();
        let renames: BTreeMap<&str, &str> = self
            .renamed
            .iter()
            .map(|(f, t)| (f.as_str(), t.as_str()))
            .collect();
        let mut moved = HashSet::new();
        for (name, _) in &mut doc.fields {
            if self.compound.contains_key(name.as_str()) {
                continue;
            }
            if let Some(to) = renames.get(name.as_str()) {
                moved.insert((name.clone(), (*to).to_owned()));
                *name = (*to).to_owned();
            }
        }
        for b in &mut doc.blocks {
            let [label] = b.labels.as_mut_slice() else {
                continue;
            };
            let Some(new_kind) = self.compound.get(label.as_str()) else {
                continue;
            };
            let to = renames[label.as_str()];
            moved.insert((label.clone(), to.to_owned()));
            *label = to.to_owned();
            b.kind = new_kind.clone();
        }
        for (from, to) in &self.renamed {
            if moved.contains(&(from.clone(), to.clone())) {
                edits.push(NoteEdit::Rename {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
        for (name, value) in &self.defaults {
            if doc.field(name).is_none() {
                doc.fields.push((name.clone(), value.clone()));
                edits.push(NoteEdit::Default {
                    field: name.clone(),
                    value: value.clone(),
                });
            }
        }
        (doc, edits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

field notes {
  kind = "content"
}

field legacy {
  kind = "string"
}

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

    fn ty(source: &str) -> TypeDef {
        TypeDef::parse("task", source).unwrap()
    }

    fn renames(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(f, t)| (f.to_string(), t.to_string()))
            .collect()
    }

    fn with(extra: &str, from: &str) -> String {
        from.replace("template default", &format!("{extra}\n\ntemplate default"))
    }

    #[test]
    fn a_new_required_field_needs_a_default() {
        let new = ty(&with(
            "field owner {\n  kind = \"string\"\n  required = true\n}",
            OLD,
        ));
        let err = FieldChanges::new(&ty(OLD), &new, &BTreeMap::new()).unwrap_err();
        assert_eq!(err.file, "types/task.wcl");
        assert_eq!(err.issues[0].at, "field owner.default");
    }

    #[test]
    fn a_new_required_field_with_a_default_fills_notes_that_lack_it() {
        let new = ty(&with(
            "field owner {\n  kind = \"string\"\n  required = true\n  default = \"me\"\n}",
            OLD,
        ));
        let changes = FieldChanges::new(&ty(OLD), &new, &BTreeMap::new()).unwrap();
        assert_eq!(changes.added, ["owner"]);
        let doc = Doc::parse("title = \"T\"\nestimate = 2\n", "n.wcl").unwrap();
        let (after, edits) = changes.apply(&doc);
        assert_eq!(after.field("owner"), Some(&Value::Str("me".into())));
        assert_eq!(edits.len(), 1);
        let (_, edits) = changes.apply(&after);
        assert!(edits.is_empty(), "a note that has the field keeps it");
    }

    #[test]
    fn a_new_optional_field_edits_nothing() {
        let new = ty(&with(
            "field owner {\n  kind = \"string\"\n  default = \"me\"\n}",
            OLD,
        ));
        let changes = FieldChanges::new(&ty(OLD), &new, &BTreeMap::new()).unwrap();
        let doc = Doc::parse("title = \"T\"\n", "n.wcl").unwrap();
        let (after, edits) = changes.apply(&doc);
        assert!(edits.is_empty());
        assert_eq!(after, doc);
    }

    #[test]
    fn renames_move_fields_and_relabel_blocks() {
        let new = ty(&OLD
            .replace("field estimate", "field points")
            .replace("field notes", "field body"));
        let changes = FieldChanges::new(
            &ty(OLD),
            &new,
            &renames(&[("estimate", "points"), ("notes", "body")]),
        )
        .unwrap();
        assert!(changes.added.is_empty() && changes.removed.is_empty());
        let doc = Doc::parse(
            "title = \"T\"\nestimate = 2\ncontent notes {\n  paragraph {\n    text = \"x\"\n  }\n}\n",
            "n.wcl",
        )
        .unwrap();
        let (after, edits) = changes.apply(&doc);
        assert_eq!(after.field("points"), Some(&Value::Int(2)));
        assert!(after.field("estimate").is_none());
        assert_eq!(after.blocks[0].labels, ["body"]);
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn a_swap_is_a_pair_of_renames() {
        let new = ty(&OLD
            .replace(
                "field estimate {\n  kind = \"int\"",
                "field legacy {\n  kind = \"int\"",
            )
            .replace(
                "field legacy {\n  kind = \"string\"",
                "field estimate {\n  kind = \"string\"",
            ));
        let changes = FieldChanges::new(
            &ty(OLD),
            &new,
            &renames(&[("estimate", "legacy"), ("legacy", "estimate")]),
        )
        .unwrap();
        let doc = Doc::parse("title = \"T\"\nestimate = 2\nlegacy = \"a\"\n", "n.wcl").unwrap();
        let (after, _) = changes.apply(&doc);
        assert_eq!(after.field("legacy"), Some(&Value::Int(2)));
        assert_eq!(after.field("estimate"), Some(&Value::Str("a".into())));
    }

    #[test]
    fn bad_renames_are_refused() {
        let old = ty(OLD);
        let cases: &[(&[(&str, &str)], &str)] = &[
            (&[("nope", "estimate")], "no field \"nope\""),
            (&[("estimate", "nope")], "no field \"nope\""),
            (&[("estimate", "legacy")], "already holds data"),
            (&[("estimate", "notes")], "cannot move a value of kind int"),
        ];
        for (pairs, expected) in cases {
            let err = FieldChanges::new(&old, &old, &renames(pairs)).unwrap_err();
            let text: Vec<String> = err.issues.iter().map(ToString::to_string).collect();
            assert!(
                text.iter().any(|t| t.contains(expected)),
                "{pairs:?}: {text:?}"
            );
        }
    }

    #[test]
    fn a_removed_field_keeps_its_data() {
        let new = ty(&OLD.replace("field legacy {\n  kind = \"string\"\n}\n\n", ""));
        let changes = FieldChanges::new(&ty(OLD), &new, &BTreeMap::new()).unwrap();
        assert_eq!(changes.removed, ["legacy"]);
        let doc = Doc::parse("title = \"T\"\nlegacy = \"keep\"\n", "n.wcl").unwrap();
        let (after, edits) = changes.apply(&doc);
        assert!(edits.is_empty());
        assert_eq!(after, doc);
    }
}
