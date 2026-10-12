//! Type files: `types/<type>.wcl`, with their fields (D3, D4, D5, D7), named templates (D14) and
//! handlers.

use std::collections::HashSet;

use crate::blocks::{CONTENT_KINDS, Check, Form, opt, req};
use crate::doc::{Doc, DocBlock, Value};
use crate::encoding::{is_name, is_ulid};
use crate::note::check_value;
use crate::{Invalid, Issue};

/// Built-in note fields. A type may not define a field with one of these names.
pub const RESERVED_FIELDS: [&str; 3] = ["title", "created", "updated"];

/// The handler events.
pub const EVENTS: [&str; 6] = [
    "before_create",
    "before_update",
    "before_delete",
    "after_create",
    "after_update",
    "after_delete",
];

/// A `list` item kind or a `dict` value kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    String,
    Int,
    Float,
    Bool,
    Date,
    Datetime,
    Url,
}

impl Scalar {
    const NAMES: [&str; 7] = ["string", "int", "float", "bool", "date", "datetime", "url"];

    fn parse(s: &str) -> Option<Scalar> {
        Some(match s {
            "string" => Scalar::String,
            "int" => Scalar::Int,
            "float" => Scalar::Float,
            "bool" => Scalar::Bool,
            "date" => Scalar::Date,
            "datetime" => Scalar::Datetime,
            "url" => Scalar::Url,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    OneWay,
    TwoWay,
    Peer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDelete {
    Unlink,
    Restrict,
    Cascade,
}

/// A reference field's definition (D4, D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The target type name, or `None` for `"*"`, any type.
    pub target: Option<String>,
    pub relation: Relation,
    /// For `two_way`: the field on the target type that holds the other side.
    pub inverse: Option<String>,
    pub on_delete: OnDelete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    String,
    Int,
    Float,
    Bool,
    Date,
    Datetime,
    /// `of = "datetime"` makes both ends datetimes.
    DateRange {
        datetime: bool,
    },
    Url,
    Select {
        options: Vec<String>,
    },
    Taxonomy {
        taxonomy: String,
    },
    Reference(Reference),
    List {
        items: Scalar,
    },
    Dict {
        values: Scalar,
    },
    /// `image` is the image asset of an image map; `None` is a real-world map.
    Location {
        image: Option<String>,
    },
    Content,
    Asset,
    Ink,
    Canvas,
}

impl FieldKind {
    /// The closed set of kind names.
    pub const NAMES: [&str; 18] = [
        "string",
        "int",
        "float",
        "bool",
        "date",
        "datetime",
        "date_range",
        "url",
        "select",
        "taxonomy",
        "reference",
        "list",
        "dict",
        "location",
        "content",
        "asset",
        "ink",
        "canvas",
    ];

    pub fn name(&self) -> &'static str {
        match self {
            FieldKind::String => "string",
            FieldKind::Int => "int",
            FieldKind::Float => "float",
            FieldKind::Bool => "bool",
            FieldKind::Date => "date",
            FieldKind::Datetime => "datetime",
            FieldKind::DateRange { .. } => "date_range",
            FieldKind::Url => "url",
            FieldKind::Select { .. } => "select",
            FieldKind::Taxonomy { .. } => "taxonomy",
            FieldKind::Reference(_) => "reference",
            FieldKind::List { .. } => "list",
            FieldKind::Dict { .. } => "dict",
            FieldKind::Location { .. } => "location",
            FieldKind::Content => "content",
            FieldKind::Asset => "asset",
            FieldKind::Ink => "ink",
            FieldKind::Canvas => "canvas",
        }
    }

    /// Written in a note as a block labelled with the field name, not as `name = value`.
    pub fn is_compound(&self) -> bool {
        matches!(
            self,
            FieldKind::Content | FieldKind::Dict { .. } | FieldKind::Ink | FieldKind::Canvas
        )
    }
}

/// Kinds that take `multiple = true`.
const MULTIPLE_KINDS: [&str; 4] = ["select", "taxonomy", "reference", "asset"];
/// Kinds that take a `default`.
const DEFAULT_KINDS: [&str; 10] = [
    "string", "int", "float", "bool", "date", "datetime", "url", "select", "taxonomy", "list",
];

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: FieldKind,
    pub required: bool,
    /// Only handlers write a computed field (D3).
    pub computed: bool,
    /// The note holds a list.
    pub multiple: bool,
    pub default: Option<Value>,
}

impl FieldDef {
    /// A single-term taxonomy field, the only kind a kanban view may group by (D7).
    pub fn is_kanban_group(&self) -> bool {
        matches!(self.kind, FieldKind::Taxonomy { .. }) && !self.multiple
    }

    pub fn reference(&self) -> Option<&Reference> {
        match &self.kind {
            FieldKind::Reference(r) => Some(r),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub name: String,
    pub label: Option<String>,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub title: String,
    pub items: Vec<TabItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabItem {
    /// Places a field, or `title`. `label` overrides the field's label here.
    Slot {
        field: String,
        label: Option<String>,
    },
    /// A fixed content block.
    Block(DocBlock),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub event: String,
    pub script: String,
    pub function: String,
    pub allow_domains: Vec<String>,
    pub secrets: Vec<String>,
}

/// One type definition.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub label: String,
    pub plural: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub colour: Option<String>,
    pub fields: Vec<FieldDef>,
    pub templates: Vec<Template>,
    pub handlers: Vec<Handler>,
}

impl TypeDef {
    /// Parse `types/<name>.wcl`. This checks the file on its own; [`crate::Schema`] checks what
    /// depends on other types and taxonomies.
    pub fn parse(name: &str, source: &str) -> Result<TypeDef, Invalid> {
        let file = format!("types/{name}.wcl");
        let doc = Doc::parse(source, &file)?;
        TypeDef::from_doc(name, &doc).map_err(|issues| Invalid { file, issues })
    }

    pub fn from_doc(name: &str, doc: &Doc) -> Result<TypeDef, Vec<Issue>> {
        let mut issues = Vec::new();
        if !is_name(name) {
            issues.push(Issue::new(
                "file",
                format!("{name:?} is not a valid type name"),
            ));
        }
        let mut c = Check {
            issues: &mut issues,
        };
        let items = [
            req("label", Form::Str),
            opt("plural", Form::Str),
            opt("description", Form::Str),
            opt("icon", Form::Str),
            opt("colour", Form::Palette),
        ];
        c.items(doc, &items, &[], "type");
        c.only_blocks(doc, &["field", "template", "handler"], "type");

        let mut fields: Vec<FieldDef> = Vec::new();
        for b in doc.blocks_of("field") {
            let at = format!("field {}", b.label().unwrap_or("?"));
            if let Some(f) = field(&mut c, b, name, &at) {
                if fields.iter().any(|g| g.name == f.name) {
                    c.fail(&at, "is defined twice");
                } else {
                    fields.push(f);
                }
            }
        }

        let mut templates: Vec<Template> = Vec::new();
        for b in doc.blocks_of("template") {
            let at = format!("template {}", b.label().unwrap_or("?"));
            if let Some(t) = template(&mut c, b, &fields, &at) {
                if templates.iter().any(|u| u.name == t.name) {
                    c.fail(&at, "is defined twice");
                } else {
                    templates.push(t);
                }
            }
        }
        if doc
            .blocks_of("template")
            .all(|b| b.label() != Some("default"))
        {
            c.fail("type", "needs a `default` template");
        }

        let handlers = doc
            .blocks_of("handler")
            .filter_map(|b| handler(&mut c, b))
            .collect();

        if !issues.is_empty() {
            return Err(issues);
        }
        let label = string(doc, "label").unwrap_or_default();
        Ok(TypeDef {
            name: name.to_owned(),
            plural: string(doc, "plural").unwrap_or_else(|| label.clone()),
            label,
            description: string(doc, "description"),
            icon: string(doc, "icon"),
            colour: string(doc, "colour"),
            fields,
            templates,
            handlers,
        })
    }

    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn template(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// The template a view asks for by name, with the fallbacks of `docs/vault-format.md`:
    /// `detail` falls back to `default`, and `card` to the first tab of `default`.
    pub fn resolve_template(&self, name: &str) -> Option<Template> {
        if let Some(t) = self.template(name) {
            return Some(t.clone());
        }
        let default = self.template("default")?;
        match name {
            "detail" => Some(default.clone()),
            "card" => Some(Template {
                name: "card".to_owned(),
                label: None,
                tabs: default.tabs.iter().take(1).cloned().collect(),
            }),
            _ => None,
        }
    }

    /// Single-term taxonomy fields: the fields a kanban view may group by (D7).
    pub fn kanban_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter().filter(|f| f.is_kanban_group())
    }
}

fn field(c: &mut Check, b: &DocBlock, type_name: &str, at: &str) -> Option<FieldDef> {
    let d = &b.body;
    let name = match b.labels.as_slice() {
        [l] if RESERVED_FIELDS.contains(&l.as_str()) => {
            c.fail(at, format!("{l:?} is a built-in field and is reserved"));
            return None;
        }
        [l] if is_name(l) => l.clone(),
        [l] => {
            c.fail(at, format!("{l:?} is not a valid field name"));
            return None;
        }
        _ => {
            c.fail(at, "needs exactly one label, the field name");
            return None;
        }
    };
    c.only_blocks(d, &[], at);

    let mut items = vec![
        req("kind", Form::OneOf(&FieldKind::NAMES)),
        opt("label", Form::Str),
        opt("description", Form::Str),
        opt("required", Form::Bool),
        opt("computed", Form::Bool),
        opt("multiple", Form::Bool),
    ];
    let kind_name = d.field("kind").and_then(Value::as_str).unwrap_or("");
    match kind_name {
        "date_range" => items.push(opt("of", Form::OneOf(&["date", "datetime"]))),
        "select" => items.push(req("options", Form::StrList)),
        "taxonomy" => items.push(req("taxonomy", Form::Str)),
        "reference" => items.extend([
            req("target", Form::Str),
            opt("relation", Form::OneOf(&["one_way", "two_way", "peer"])),
            opt("inverse", Form::Str),
            opt("on_delete", Form::OneOf(&["unlink", "restrict", "cascade"])),
        ]),
        "list" => items.push(req("items", Form::OneOf(&Scalar::NAMES))),
        "dict" => items.push(req("values", Form::OneOf(&Scalar::NAMES))),
        "location" => items.extend([
            req("map", Form::OneOf(&["geo", "image"])),
            opt("image", Form::Ulid),
        ]),
        _ => {}
    }
    let before = c.issues.len();
    c.items(d, &items, &["default"], at);
    if c.issues.len() > before || !FieldKind::NAMES.contains(&kind_name) {
        return None;
    }

    let flag = |n: &str| d.field(n) == Some(&Value::Bool(true));
    let s = |n: &str| string(d, n);
    let (required, computed, multiple) = (flag("required"), flag("computed"), flag("multiple"));

    let kind = match kind_name {
        "string" => FieldKind::String,
        "int" => FieldKind::Int,
        "float" => FieldKind::Float,
        "bool" => FieldKind::Bool,
        "date" => FieldKind::Date,
        "datetime" => FieldKind::Datetime,
        "date_range" => FieldKind::DateRange {
            datetime: s("of").as_deref() == Some("datetime"),
        },
        "url" => FieldKind::Url,
        "select" => {
            let options: Vec<String> = match d.field("options") {
                Some(Value::List(l)) => l
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                _ => Vec::new(),
            };
            if options.is_empty() {
                c.fail(&format!("{at}.options"), "needs at least one option");
            }
            let mut seen = HashSet::new();
            for o in &options {
                if !seen.insert(o) {
                    c.fail(&format!("{at}.options"), format!("lists {o:?} twice"));
                }
            }
            FieldKind::Select { options }
        }
        "taxonomy" => {
            let taxonomy = s("taxonomy").unwrap_or_default();
            if !is_name(&taxonomy) {
                c.fail(
                    &format!("{at}.taxonomy"),
                    format!("{taxonomy:?} is not a valid taxonomy name"),
                );
            }
            FieldKind::Taxonomy { taxonomy }
        }
        "reference" => FieldKind::Reference(reference(c, d, &name, type_name, multiple, at)?),
        "list" => FieldKind::List {
            items: Scalar::parse(&s("items").unwrap_or_default())?,
        },
        "dict" => FieldKind::Dict {
            values: Scalar::parse(&s("values").unwrap_or_default())?,
        },
        "location" => {
            let image = s("image");
            match (s("map").as_deref(), &image) {
                (Some("image"), None) => {
                    c.fail(&format!("{at}.image"), "is required when map is \"image\"")
                }
                (Some("geo"), Some(_)) => {
                    c.fail(&format!("{at}.image"), "is used only when map is \"image\"")
                }
                _ => {}
            }
            FieldKind::Location {
                image: image.filter(|i| is_ulid(i)),
            }
        }
        "content" => FieldKind::Content,
        "asset" => FieldKind::Asset,
        "ink" => FieldKind::Ink,
        "canvas" => FieldKind::Canvas,
        _ => return None,
    };

    if multiple && !MULTIPLE_KINDS.contains(&kind_name) {
        c.fail(
            &format!("{at}.multiple"),
            format!("is not allowed on a {kind_name} field"),
        );
    }
    if computed && required {
        c.fail(at, "a computed field cannot also be required");
    }

    let def = FieldDef {
        label: s("label").unwrap_or_else(|| name.clone()),
        description: s("description"),
        name,
        kind,
        required,
        computed,
        multiple,
        default: d.field("default").cloned(),
    };
    if let Some(v) = &def.default {
        let path = format!("{at}.default");
        if !DEFAULT_KINDS.contains(&kind_name) {
            c.fail(&path, format!("is not allowed on a {kind_name} field"));
        } else {
            check_value(&def, v, None, c, &path);
        }
    }
    Some(def)
}

fn reference(
    c: &mut Check,
    d: &Doc,
    field: &str,
    type_name: &str,
    multiple: bool,
    at: &str,
) -> Option<Reference> {
    let target = string(d, "target").unwrap_or_default();
    let relation = match string(d, "relation").as_deref() {
        Some("two_way") => Relation::TwoWay,
        Some("peer") => Relation::Peer,
        _ => Relation::OneWay,
    };
    let on_delete = match string(d, "on_delete").as_deref() {
        Some("restrict") => OnDelete::Restrict,
        Some("cascade") => OnDelete::Cascade,
        _ => OnDelete::Unlink,
    };
    let inverse = string(d, "inverse");
    let target = if target == "*" {
        if relation != Relation::OneWay {
            c.fail(
                &format!("{at}.target"),
                "\"*\" is allowed only with relation \"one_way\"",
            );
        }
        None
    } else if is_name(&target) {
        Some(target)
    } else {
        c.fail(
            &format!("{at}.target"),
            format!("{target:?} is not a type name or \"*\""),
        );
        return None;
    };
    match relation {
        Relation::TwoWay => match &inverse {
            None => c.fail(
                &format!("{at}.inverse"),
                "is required for a two_way reference",
            ),
            Some(i) if !is_name(i) => c.fail(
                &format!("{at}.inverse"),
                format!("{i:?} is not a field name"),
            ),
            Some(i) if target.as_deref() == Some(type_name) && i == field => {
                c.fail(&format!("{at}.inverse"), "cannot name the field itself")
            }
            _ => {}
        },
        _ if inverse.is_some() => {
            c.fail(
                &format!("{at}.inverse"),
                "is used only by a two_way reference",
            );
        }
        _ => {}
    }
    if relation == Relation::Peer {
        if !multiple {
            c.fail(at, "a peer reference must be multiple");
        }
        if target.as_deref() != Some(type_name) {
            c.fail(
                &format!("{at}.target"),
                format!("a peer reference must target its own type, {type_name:?}"),
            );
        }
    }
    Some(Reference {
        target,
        relation,
        inverse,
        on_delete,
    })
}

fn template(c: &mut Check, b: &DocBlock, fields: &[FieldDef], at: &str) -> Option<Template> {
    let name = c.ident_label(b, at)?.to_owned();
    let d = &b.body;
    c.items(d, &[opt("label", Form::Str)], &[], at);
    c.only_blocks(d, &["tab"], at);
    if d.blocks_of("tab").next().is_none() {
        c.fail(at, "needs at least one tab");
    }
    let mut placed = HashSet::new();
    let mut tabs = Vec::new();
    for t in d.blocks_of("tab") {
        let tab_at = format!("{at} > {}", t.name());
        let title = match t.labels.as_slice() {
            [l] => l.clone(),
            _ => {
                c.fail(&tab_at, "needs exactly one label, the tab title");
                continue;
            }
        };
        c.items(&t.body, &[], &[], &tab_at);
        let mut items = Vec::new();
        for (n, block) in t.body.blocks.iter().enumerate() {
            if block.kind == "slot" {
                let slot_at = format!("{tab_at} > {}", block.name());
                let [field] = block.labels.as_slice() else {
                    c.fail(&slot_at, "needs exactly one label, the field name");
                    continue;
                };
                if field != "title" && !fields.iter().any(|f| &f.name == field) {
                    c.fail(&slot_at, format!("the type has no field {field:?}"));
                } else if !placed.insert(field.clone()) {
                    c.fail(
                        &slot_at,
                        format!("places {field:?} a second time in this template"),
                    );
                }
                c.items(&block.body, &[opt("label", Form::Str)], &[], &slot_at);
                c.only_blocks(&block.body, &[], &slot_at);
                items.push(TabItem::Slot {
                    field: field.clone(),
                    label: string(&block.body, "label"),
                });
            } else if CONTENT_KINDS.contains(&block.kind.as_str()) {
                let block_at = format!("{tab_at} > {}#{}", block.kind, n + 1);
                c.no_label(block, &block_at);
                c.content_block(block, &block_at);
                items.push(TabItem::Block(block.clone()));
            } else {
                c.fail(
                    &format!("{tab_at} > {}", block.name()),
                    "is neither a slot nor a content block",
                );
            }
        }
        tabs.push(Tab { title, items });
    }
    Some(Template {
        name,
        label: string(d, "label"),
        tabs,
    })
}

fn handler(c: &mut Check, b: &DocBlock) -> Option<Handler> {
    let event = b.label().unwrap_or("").to_owned();
    let at = format!("handler {event:?}");
    if b.labels.len() != 1 || !EVENTS.contains(&event.as_str()) {
        c.fail(
            &at,
            format!("the label must be one event: {}", EVENTS.join(", ")),
        );
    }
    let d = &b.body;
    let items = [
        req("script", Form::Str),
        opt("function", Form::Ident),
        opt("allow_domains", Form::StrList),
        opt("secrets", Form::StrList),
    ];
    c.items(d, &items, &[], &at);
    c.only_blocks(d, &[], &at);
    let script = string(d, "script").unwrap_or_default();
    if d.field("script").is_some() && !is_name(&script) {
        c.fail(
            &format!("{at}.script"),
            format!("{script:?} is not a valid script name"),
        );
    }
    if event.starts_with("before_") {
        for n in ["allow_domains", "secrets"] {
            if d.field(n).is_some() {
                c.fail(&format!("{at}.{n}"), "is allowed only on after_* handlers");
            }
        }
    }
    let list = |n: &str| match d.field(n) {
        Some(Value::List(l)) => l
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    Some(Handler {
        function: string(d, "function").unwrap_or_else(|| event.clone()),
        allow_domains: list("allow_domains"),
        secrets: list("secrets"),
        script,
        event,
    })
}

pub(crate) fn string(doc: &Doc, name: &str) -> Option<String> {
    doc.field(name).and_then(Value::as_str).map(str::to_owned)
}
