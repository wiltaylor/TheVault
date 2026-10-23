//! Type, field, relationship, taxonomy and template definitions, note validation and schema-change planning.
//!
//! This is the semantic layer of `docs/vault-format.md` "Validating vault files": what WCL cannot
//! express. [`TypeDef::parse`], [`Taxonomy::parse`] and [`Agent::parse`] check one definition
//! file each, and [`Schema::new`] checks the rules between them: reference targets and inverses,
//! and the taxonomies fields name. [`Schema::validate_note`] checks a note against its type, the notes it
//! references and the terms it holds.

mod agents;
mod blocks;
mod change;
mod devices;
mod doc;
mod encoding;
mod inline;
pub mod migrate;
mod note;
mod queryviews;
mod taxonomy;
mod types;

use std::collections::BTreeMap;
use std::fmt;

pub use agents::{
    AGENT_EFFORTS, AGENT_PROMPT_MAX, AGENT_PROVIDERS, ASSISTANT_AGENT, Agent, CONFIRM_SWITCHES,
    Confirm as AgentConfirm, IMPLICIT_MENTION, Mention as MentionTrigger, NoteEvent,
    OUTBOX_SWITCHES, Outbox as AgentOutbox, Permissions as AgentPermissions, Schedule, TRIGGER_OPS,
    Trigger, WEB_SWITCHES, Web as AgentWeb, When as TriggerWhen, parse_cron,
};
pub use blocks::{
    ADMONITION_KINDS, ARROWS, AdmonitionPreset, CANVAS_HEIGHT_DEFAULT, CANVAS_HEIGHT_MAX,
    CANVAS_HEIGHT_MIN, CARD_BLOCK_KINDS, COLUMNS_MAX, COLUMNS_MIN, CONTENT_KINDS, CONTROL_KINDS,
    INK_HEIGHT_DEFAULT, INK_HEIGHT_MAX, INK_HEIGHT_MIN, MAP_VIEW_HEIGHT_DEFAULT,
    MAP_VIEW_HEIGHT_MAX, MAP_VIEW_HEIGHT_MIN, NODE_KINDS, RATING_MAX_DEFAULT, RATING_MAX_LIMIT,
    VIEW_KINDS, admonition_preset, control_fits, is_icon,
};
pub use change::{FieldChanges, NoteEdit};
pub use devices::{DEVICE_COMMANDS, DEVICE_EVENTS, DEVICE_PLATFORMS, DeviceFile, DeviceHook};
pub use doc::{Doc, DocBlock, Value};
pub use encoding::{
    PALETTE, is_date, is_datetime, is_duration, is_ident, is_ink_colour, is_name, is_palette,
    is_ulid, is_url, is_utc_datetime, parse_datetime,
};
pub use inline::{
    AGENT_CALL_SCHEME, AgentCall, MARKUP_ITEMS, Placeholder, agent_calls, escape_markup,
    markup_item, parse_agent_call, placeholder_at, placeholders,
};
pub use note::{NoteLookup, NoteReport};
pub use queryviews::{FieldRename, VIEW_FIELD_ITEMS, query_view_issues, reads_type};
pub use taxonomy::{Taxonomy, Term};
pub use types::{
    Accept, EVENTS, FieldDef, FieldKind, Handler, LIST_PAGE_SIZE_MAX, LIST_PAGE_SIZE_MIN,
    LIST_PAGE_VIEWS, ListPage, OnDelete, RESERVED_FIELDS, RangeOf, Reference, Relation, Scalar,
    Tab, TabItem, Template, TypeDef, is_image_filename,
};

/// How much an [`Issue`] matters. An error makes the file invalid. A warning does not: it reports
/// something this version ignores, such as an attribute of a type or field that only a newer
/// release knows, so that a newer server's files still load in an older app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Severity {
    #[default]
    Error,
    Warning,
}

/// One problem, at a path inside a file such as `field estimate.default` or
/// `content body > list#2 > item#1.text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub at: String,
    pub message: String,
    pub severity: Severity,
    /// The id of the note the issue says does not exist, for a reference to a missing note. A
    /// device that holds that note pushes it first (ticket 150).
    pub missing_note: Option<String>,
}

impl Issue {
    /// An error.
    pub fn new(at: impl Into<String>, message: impl Into<String>) -> Issue {
        Issue {
            at: at.into(),
            message: message.into(),
            severity: Severity::Error,
            missing_note: None,
        }
    }

    /// An error for a reference to note `id`, which does not exist.
    pub fn missing_note(at: impl Into<String>, id: &str) -> Issue {
        Issue {
            missing_note: Some(id.to_owned()),
            ..Issue::new(at, format!("note {id} does not exist"))
        }
    }

    /// A warning, which leaves the file valid.
    pub fn warning(at: impl Into<String>, message: impl Into<String>) -> Issue {
        Issue {
            severity: Severity::Warning,
            ..Issue::new(at, message)
        }
    }

    pub fn is_warning(&self) -> bool {
        self.severity == Severity::Warning
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_warning() {
            write!(f, "warning: ")?;
        }
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// A vault file that failed validation, with every issue found in it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {}", .issues.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct Invalid {
    pub file: String,
    pub issues: Vec<Issue>,
}

/// Every definition file that failed, from [`Schema::new`] or [`Schema::load`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct SchemaError(pub Vec<Invalid>);

/// A vault's types and taxonomies, checked against each other.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    types: BTreeMap<String, TypeDef>,
    taxonomies: BTreeMap<String, Taxonomy>,
}

impl Schema {
    /// Check the definitions against each other and build the schema.
    pub fn new(types: Vec<TypeDef>, taxonomies: Vec<Taxonomy>) -> Result<Schema, SchemaError> {
        let schema = Schema {
            types: types.into_iter().map(|t| (t.name.clone(), t)).collect(),
            taxonomies: taxonomies
                .into_iter()
                .map(|t| (t.name.clone(), t))
                .collect(),
        };
        let failed: Vec<Invalid> = schema
            .types
            .values()
            .filter_map(|t| {
                let issues = schema.cross_check(t);
                (!issues.is_empty()).then(|| Invalid {
                    file: format!("types/{}.wcl", t.name),
                    issues,
                })
            })
            .collect();
        if failed.is_empty() {
            Ok(schema)
        } else {
            Err(SchemaError(failed))
        }
    }

    /// Parse and check definition sources, given as `(name, source)` pairs.
    pub fn from_sources(
        types: &[(&str, &str)],
        taxonomies: &[(&str, &str)],
    ) -> Result<Schema, SchemaError> {
        let mut failed = Vec::new();
        let types = types
            .iter()
            .filter_map(|(n, s)| TypeDef::parse(n, s).map_err(|e| failed.push(e)).ok())
            .collect();
        let taxonomies = taxonomies
            .iter()
            .filter_map(|(n, s)| Taxonomy::parse(n, s).map_err(|e| failed.push(e)).ok())
            .collect();
        if !failed.is_empty() {
            return Err(SchemaError(failed));
        }
        Schema::new(types, taxonomies)
    }

    /// Read `types/*.wcl` and `taxonomies/*.wcl` from a vault.
    pub fn load(vault: &store::Vault) -> Result<Schema, SchemaError> {
        let read = |dir: &str| -> Result<Vec<(String, String)>, SchemaError> {
            let fail = |file: String, message: String| {
                SchemaError(vec![Invalid {
                    file,
                    issues: vec![Issue::new("file", message)],
                }])
            };
            let entries = std::fs::read_dir(vault.root().join(dir))
                .map_err(|e| fail(dir.to_owned(), e.to_string()))?;
            let mut out = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| fail(dir.to_owned(), e.to_string()))?;
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let Some(stem) = file_name.strip_suffix(".wcl") else {
                    continue;
                };
                let path = format!("{dir}/{file_name}");
                let text = vault
                    .read_text(&path)
                    .map_err(|e| fail(path.clone(), e.to_string()))?;
                out.push((stem.to_owned(), text));
            }
            out.sort();
            Ok(out)
        };
        let types = read("types")?;
        let taxonomies = read("taxonomies")?;
        Schema::from_sources(&borrow(&types), &borrow(&taxonomies))
    }

    pub fn type_def(&self, name: &str) -> Option<&TypeDef> {
        self.types.get(name)
    }

    pub fn types(&self) -> impl Iterator<Item = &TypeDef> {
        self.types.values()
    }

    pub fn taxonomy(&self, name: &str) -> Option<&Taxonomy> {
        self.taxonomies.get(name)
    }

    pub fn taxonomies(&self) -> impl Iterator<Item = &Taxonomy> {
        self.taxonomies.values()
    }

    /// Put `taxonomy` in place of the one of its name. Only for a change that keeps every term,
    /// such as adding one, which cannot break the checks between definitions.
    pub fn grow_taxonomy(&mut self, taxonomy: Taxonomy) {
        self.taxonomies.insert(taxonomy.name.clone(), taxonomy);
    }

    /// Whether a kanban view may group notes of `type_name` by `field`: a single-term taxonomy
    /// field (D7).
    pub fn is_kanban_group(&self, type_name: &str, field: &str) -> bool {
        self.type_def(type_name)
            .and_then(|t| t.field(field))
            .is_some_and(FieldDef::is_kanban_group)
    }

    /// Parse a note's source and validate it against `type_name`.
    pub fn validate_note_source(
        &self,
        type_name: &str,
        source: &str,
        file: &str,
        lookup: &dyn NoteLookup,
    ) -> Result<NoteReport, Invalid> {
        let doc = Doc::parse(source, file)?;
        self.validate_note(type_name, &doc, lookup)
            .map_err(|issues| Invalid {
                file: file.to_owned(),
                issues,
            })
    }

    /// Validate a note against its type. On success the report names any item the type does not
    /// describe, which flags the note without rejecting it (D6).
    pub fn validate_note(
        &self,
        type_name: &str,
        doc: &Doc,
        lookup: &dyn NoteLookup,
    ) -> Result<NoteReport, Vec<Issue>> {
        note::validate(self, type_name, doc, lookup)
    }

    /// The rules between one type and the rest of the schema.
    fn cross_check(&self, t: &TypeDef) -> Vec<Issue> {
        let mut issues = Vec::new();
        for f in &t.fields {
            let at = format!("field {}", f.name);
            match &f.kind {
                FieldKind::Taxonomy { taxonomy } => match self.taxonomy(taxonomy) {
                    None => issues.push(Issue::new(
                        format!("{at}.taxonomy"),
                        format!("there is no taxonomy {taxonomy:?}"),
                    )),
                    Some(tax) => {
                        let defaults = match &f.default {
                            Some(Value::Str(s)) => vec![s.as_str()],
                            Some(Value::List(l)) => l.iter().filter_map(Value::as_str).collect(),
                            _ => Vec::new(),
                        };
                        for d in defaults.into_iter().filter(|d| !tax.contains(d)) {
                            issues.push(Issue::new(
                                format!("{at}.default"),
                                format!("{d:?} is not a term of taxonomy {taxonomy:?}"),
                            ));
                        }
                    }
                },
                FieldKind::Reference(r) => {
                    let Some(target) = &r.target else { continue };
                    let Some(target_type) = self.type_def(target) else {
                        issues.push(Issue::new(
                            format!("{at}.target"),
                            format!("there is no type {target:?}"),
                        ));
                        continue;
                    };
                    let Some(inverse) = &r.inverse else { continue };
                    let other = target_type.field(inverse);
                    let ok = other.and_then(FieldDef::reference).is_some_and(|o| {
                        o.relation == Relation::TwoWay
                            && o.target.as_deref() == Some(t.name.as_str())
                            && o.inverse.as_deref() == Some(f.name.as_str())
                    });
                    if !ok {
                        issues.push(Issue::new(
                            format!("{at}.inverse"),
                            format!(
                                "{target}.{inverse} must be a two_way reference to {} whose inverse is {:?}",
                                t.name, f.name
                            ),
                        ));
                    }
                }
                _ => {}
            }
        }
        for tpl in &t.templates {
            for (n, tab) in tpl.tabs.iter().enumerate() {
                for item in &tab.items {
                    let TabItem::Block(b) = item else { continue };
                    let mut found = Vec::new();
                    let at = format!("template {} > tab#{}", tpl.name, n + 1);
                    inline::placeholders_in(std::slice::from_ref(b), &at, &mut found);
                    for (path, p) in found {
                        if self.type_def(&p.type_name).is_none() {
                            issues.push(Issue::new(path, note::unknown_placeholder_type(&p)));
                        }
                    }
                }
            }
        }
        issues
    }
}

/// A [`NoteLookup`] over a map of note id to type name, in which every asset exists.
impl NoteLookup for std::collections::HashMap<String, String> {
    fn note_type(&self, id: &str) -> Option<String> {
        self.get(id).cloned()
    }

    fn asset_exists(&self, _id: &str) -> bool {
        true
    }
}

fn borrow(v: &[(String, String)]) -> Vec<(&str, &str)> {
    v.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect()
}
