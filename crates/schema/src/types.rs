//! Type files: `types/<type>.wcl`, with their fields (D3, D4, D5, D7), named templates (D14) and
//! handlers.

use std::collections::HashSet;

use query_syntax::Expr;

use crate::blocks::{CONTENT_KINDS, Check, Form, opt, req};
use crate::doc::{Doc, DocBlock, Value};
use crate::encoding::{is_name, is_ulid};
use crate::note::check_value;
use crate::{Invalid, Issue};

/// Built-in note fields. A type may not define a field with one of these names. `archived` is
/// the archive flag, which the server writes, and `conflict_of` names the note a sync conflict
/// copy was made from (see `docs/vault-format.md` "Note files").
pub const RESERVED_FIELDS: [&str; 5] = ["title", "created", "updated", "archived", "conflict_of"];

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

/// The `items` of a `list` field: a scalar kind, or `taxonomy` for a list of taxonomy terms.
const ITEM_NAMES: [&str; 8] = [
    "string", "int", "float", "bool", "date", "datetime", "url", "taxonomy",
];

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
    /// `of` says what both ends are.
    DateRange {
        of: RangeOf,
    },
    Url,
    /// `option_labels` is empty, or holds one label per option, index-aligned. An empty label
    /// shows the option's value.
    Select {
        options: Vec<String>,
        option_labels: Vec<String>,
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

/// What the ends of a date range are: both dates, both datetimes, or either form as long as both
/// ends share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOf {
    Date,
    Datetime,
    DateOrDatetime,
}

impl RangeOf {
    /// The closed set of `of` values.
    pub const NAMES: [&str; 3] = ["date", "datetime", "date_or_datetime"];

    pub fn name(self) -> &'static str {
        match self {
            RangeOf::Date => "date",
            RangeOf::Datetime => "datetime",
            RangeOf::DateOrDatetime => "date_or_datetime",
        }
    }

    fn parse(s: Option<&str>) -> RangeOf {
        match s {
            Some("datetime") => RangeOf::Datetime,
            Some("date_or_datetime") => RangeOf::DateOrDatetime,
            _ => RangeOf::Date,
        }
    }
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
    /// Kept off the note page and the properties sidebar. The value is still stored, queried and
    /// written through the API and scripts.
    pub hidden: bool,
    /// The note holds a list.
    pub multiple: bool,
    /// Written as `kind = "list"` with `items = "taxonomy"`. Such a field is read as a `taxonomy`
    /// field with `multiple = true` everywhere else: in notes, queries and the index.
    pub term_list: bool,
    pub default: Option<Value>,
    /// On a list of strings: another list-of-string field of the type that holds one label per
    /// value, index-aligned. The named field is edited with this one and not shown on its own.
    pub labels: Option<String>,
    /// On an asset field: which assets it may hold.
    pub accept: Accept,
    /// Attributes this version does not know, kept as written so an editor that rewrites the
    /// field can keep them (see [`TypeDef::warnings`]).
    pub extra: Vec<(String, Value)>,
}

/// The `accept` of an asset field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accept {
    #[default]
    Any,
    /// Only an asset whose file is an image, by its file name.
    Image,
}

impl Accept {
    pub fn name(self) -> &'static str {
        match self {
            Accept::Any => "any",
            Accept::Image => "image",
        }
    }
}

/// Whether a file name names an image: `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.avif` or
/// `.svg`, in any case.
pub fn is_image_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg"
        )
    })
}

/// The built-in title, as a required string field, for what may name it as a field: a slot or
/// a control.
pub(crate) fn title_field() -> FieldDef {
    FieldDef {
        name: "title".to_owned(),
        label: "Title".to_owned(),
        description: None,
        kind: FieldKind::String,
        required: true,
        computed: false,
        hidden: false,
        multiple: false,
        term_list: false,
        default: None,
        labels: None,
        accept: Accept::Any,
        extra: Vec::new(),
    }
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

    /// The text a select field shows for option `value`: its label, else the value itself.
    pub fn option_text<'a>(&'a self, value: &'a str) -> &'a str {
        match &self.kind {
            FieldKind::Select {
                options,
                option_labels,
            } => options
                .iter()
                .position(|o| o == value)
                .and_then(|n| option_labels.get(n))
                .filter(|l| !l.is_empty())
                .map_or(value, String::as_str),
            _ => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub name: String,
    pub label: Option<String>,
    pub tabs: Vec<Tab>,
    /// Attributes this version does not know, kept as written.
    pub extra: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub title: String,
    /// The condition, in the syntax of a query's `where` clause, under which the tab shows for a
    /// note. `None` shows it for every note. The web client evaluates it as the note is edited.
    pub when: Option<String>,
    pub items: Vec<TabItem>,
    /// Attributes this version does not know, kept as written.
    pub extra: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabItem {
    /// Places a field, or `title`. `label` overrides the field's label here. `extra` holds the
    /// attributes this version does not know, kept as written.
    Slot {
        field: String,
        label: Option<String>,
        extra: Vec<(String, Value)>,
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

/// The views a type's list page shows its notes in (lists L1, L4).
pub const LIST_PAGE_VIEWS: [&str; 3] = ["cards", "list", "details"];

/// The fewest and the most notes one page of a type's list page shows (lists L1, L2).
pub const LIST_PAGE_SIZE_MIN: i64 = 10;
pub const LIST_PAGE_SIZE_MAX: i64 = 500;

/// The built-in note fields a list page may sort by, beside the type's own fields.
const LIST_PAGE_SORT_BUILT_INS: [&str; 3] = ["title", "created", "updated"];

/// A type's `list_page` block: how its list page (`/types/<type>`) shows its notes when the user
/// has chosen nothing (lists L1). Each item is `None` when the block leaves it out; the reader
/// of the settings applies the defaults: `cards`, `updated desc` and 50.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPage {
    /// `cards`, `list` or `details`.
    pub view: Option<String>,
    /// A field of the type, or `title`, `created` or `updated`, and whether it sorts descending.
    pub sort: Option<(String, bool)>,
    /// From [`LIST_PAGE_SIZE_MIN`] to [`LIST_PAGE_SIZE_MAX`].
    pub page_size: Option<i64>,
}

impl ListPage {
    /// The `sort` item as the file writes it, such as `sent desc`.
    pub fn sort_text(&self) -> Option<String> {
        self.sort
            .as_ref()
            .map(|(f, desc)| format!("{f} {}", if *desc { "desc" } else { "asc" }))
    }
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
    /// The `list_page` block, when the file has one (lists L1).
    pub list_page: Option<ListPage>,
    /// What this version ignores in the file, such as an attribute only a newer release knows.
    pub warnings: Vec<Issue>,
    /// The type's own attributes this version does not know, kept as written. Each field,
    /// template, tab and slot keeps its own, so an editor that rewrites the file from this
    /// definition loses none of them.
    pub extra: Vec<(String, Value)>,
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
            fields: None,
            template: false,
        };
        let items = [
            req("label", Form::Str),
            opt("plural", Form::Str),
            opt("description", Form::Str),
            opt("icon", Form::Str),
            opt("colour", Form::Palette),
        ];
        c.attributes(doc, &items, &[], "type", "types");
        let extra = unknown(doc, &items, &[]);
        c.only_blocks(doc, &["field", "template", "handler", "list_page"], "type");

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

        check_labels(&mut c, &fields);

        // A control in a template binds a field of this type.
        c.fields = Some(&fields);
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

        let mut list_page = None;
        for (k, b) in doc.blocks_of("list_page").enumerate() {
            if k > 0 {
                c.fail(
                    "list_page",
                    "is defined twice: a type has one list_page block",
                );
            } else {
                list_page = Some(list_page_of(&mut c, b, &fields));
            }
        }

        // A type with an error is refused, and reports only its errors. Warnings leave it valid
        // and stay with the definition, so the type editor and `PUT /api/types` can show them.
        let (warnings, errors): (Vec<Issue>, Vec<Issue>) =
            issues.into_iter().partition(Issue::is_warning);
        if !errors.is_empty() {
            return Err(errors);
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
            list_page,
            warnings,
            extra,
        })
    }

    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn template(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// The template a view asks for by name, with the fallback of `docs/vault-format.md`: `card`
    /// falls back to the first tab of `default`.
    pub fn resolve_template(&self, name: &str) -> Option<Template> {
        if let Some(t) = self.template(name) {
            return Some(t.clone());
        }
        let default = self.template("default")?;
        match name {
            "card" => Some(Template {
                name: "card".to_owned(),
                label: None,
                tabs: default.tabs.iter().take(1).cloned().collect(),
                extra: Vec::new(),
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
        opt("hidden", Form::Bool),
        opt("multiple", Form::Bool),
    ];
    let kind_name = d.field("kind").and_then(Value::as_str).unwrap_or("");
    match kind_name {
        "date_range" => items.push(opt("of", Form::OneOf(&RangeOf::NAMES))),
        "select" => items.extend([
            req("options", Form::StrList),
            opt("option_labels", Form::StrList),
        ]),
        "taxonomy" => items.push(req("taxonomy", Form::Str)),
        "reference" => items.extend([
            req("target", Form::Str),
            opt("relation", Form::OneOf(&["one_way", "two_way", "peer"])),
            opt("inverse", Form::Str),
            opt("on_delete", Form::OneOf(&["unlink", "restrict", "cascade"])),
        ]),
        "list" => {
            items.push(req("items", Form::OneOf(&ITEM_NAMES)));
            match d.field("items").and_then(Value::as_str) {
                Some("taxonomy") => items.push(req("taxonomy", Form::Str)),
                Some("string") => items.push(opt("labels", Form::Str)),
                _ => {}
            }
        }
        "asset" => items.push(opt("accept", Form::OneOf(&["image", "any"]))),
        "dict" => items.push(req("values", Form::OneOf(&Scalar::NAMES))),
        "location" => items.extend([
            req("map", Form::OneOf(&["geo", "image"])),
            opt("image", Form::Ulid),
        ]),
        _ => {}
    }
    let before = c.errors();
    let owner = if FieldKind::NAMES.contains(&kind_name) {
        format!("{kind_name} fields")
    } else {
        "fields".to_owned()
    };
    c.attributes(d, &items, &["default"], at, &owner);
    if c.errors() > before || !FieldKind::NAMES.contains(&kind_name) {
        return None;
    }

    let known = |n: &str| items.iter().any(|i| i.name == n);
    let flag = |n: &str| d.field(n) == Some(&Value::Bool(true));
    let s = |n: &str| string(d, n);
    let (required, computed, multiple) = (flag("required"), flag("computed"), flag("multiple"));
    let hidden = flag("hidden");
    let term_list = kind_name == "list" && s("items").as_deref() == Some("taxonomy");

    let kind = match kind_name {
        "string" => FieldKind::String,
        "int" => FieldKind::Int,
        "float" => FieldKind::Float,
        "bool" => FieldKind::Bool,
        "date" => FieldKind::Date,
        "datetime" => FieldKind::Datetime,
        "date_range" => FieldKind::DateRange {
            of: RangeOf::parse(s("of").as_deref()),
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
            let option_labels: Vec<String> = match d.field("option_labels") {
                Some(Value::List(l)) => l
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                _ => Vec::new(),
            };
            if d.field("option_labels").is_some() && option_labels.len() != options.len() {
                c.fail(
                    &format!("{at}.option_labels"),
                    format!(
                        "holds {} labels for {} options: give one label per option, in the same order",
                        option_labels.len(),
                        options.len()
                    ),
                );
            }
            FieldKind::Select {
                options,
                option_labels,
            }
        }
        "taxonomy" | "list" if kind_name == "taxonomy" || term_list => {
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
    if hidden && required {
        c.fail(at, "a hidden field cannot also be required");
    }

    let def = FieldDef {
        label: s("label").unwrap_or_else(|| name.clone()),
        description: s("description"),
        name,
        kind,
        required,
        computed,
        hidden,
        // A list of terms holds several terms, as a multiple taxonomy field does.
        multiple: multiple || term_list,
        term_list,
        default: d.field("default").cloned(),
        // An attribute this kind does not have is ignored, as a newer release's would be.
        labels: s("labels").filter(|_| known("labels")),
        accept: if known("accept") && s("accept").as_deref() == Some("image") {
            Accept::Image
        } else {
            Accept::Any
        },
        extra: unknown(d, &items, &["default"]),
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

/// Every `labels` names another list-of-string field of the type, and no field is named by two.
fn check_labels(c: &mut Check, fields: &[FieldDef]) {
    let mut named: Vec<(&str, &str)> = Vec::new();
    for f in fields {
        let Some(l) = &f.labels else { continue };
        let at = format!("field {}.labels", f.name);
        if l == &f.name {
            c.fail(&at, "cannot name the field itself");
            continue;
        }
        match fields.iter().find(|g| &g.name == l) {
            None => c.fail(&at, format!("the type has no field {l:?}")),
            Some(g)
                if !matches!(
                    g.kind,
                    FieldKind::List {
                        items: Scalar::String
                    }
                ) =>
            {
                c.fail(
                    &at,
                    format!(
                        "{l:?} must be a list of strings, and it is a {} field",
                        g.kind.name()
                    ),
                )
            }
            Some(_) => {
                if let Some((other, _)) = named.iter().find(|(_, n)| *n == l.as_str()) {
                    c.fail(
                        &at,
                        format!("{l:?} already holds the labels of field {other:?}"),
                    );
                } else {
                    named.push((f.name.as_str(), l.as_str()));
                }
            }
        }
    }
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

/// The only templates a type may have: `default` for the note page, `card` for cards, kanban,
/// map pins and canvas nodes. Both may have tabs.
pub const TEMPLATE_NAMES: [&str; 2] = ["default", "card"];

fn template(c: &mut Check, b: &DocBlock, fields: &[FieldDef], at: &str) -> Option<Template> {
    let name = c.ident_label(b, at)?.to_owned();
    if !TEMPLATE_NAMES.contains(&name.as_str()) {
        c.fail(
            at,
            format!("{name:?} is not a template a type can have: only `default` and `card`"),
        );
        return None;
    }
    let d = &b.body;
    let template_items = [opt("label", Form::Str)];
    c.attributes(d, &template_items, &[], at, "templates");
    c.only_blocks(d, &["tab"], at);
    if d.blocks_of("tab").next().is_none() {
        c.fail(at, "needs at least one tab");
    }
    let mut placed = HashSet::new();
    let mut tabs = Vec::new();
    // A template's fixed blocks may show the note's own values: `{{name}}`, an image's `field`.
    c.template = true;
    for t in d.blocks_of("tab") {
        let tab_at = format!("{at} > {}", t.name());
        let title = match t.labels.as_slice() {
            [l] => l.clone(),
            _ => {
                c.fail(&tab_at, "needs exactly one label, the tab title");
                continue;
            }
        };
        let tab_items = [opt("when", Form::Str)];
        c.attributes(&t.body, &tab_items, &[], &tab_at, "tabs");
        let when = string(&t.body, "when");
        if let Some(w) = &when {
            check_when(c, w, fields, &format!("{tab_at}.when"));
        }
        let mut items = Vec::new();
        for (n, block) in t.body.blocks.iter().enumerate() {
            if block.kind == "slot" {
                let slot_at = format!("{tab_at} > {}", block.name());
                if let Some(item) = slot(c, block, fields, &mut placed, &slot_at) {
                    items.push(item);
                }
            } else if block.kind == "columns" {
                // A layout of slots and fixed blocks side by side: each column is checked as a
                // tab is, and a slot in a column places its field as one in the tab would.
                let block_at = format!("{tab_at} > {}#{}", block.kind, n + 1);
                c.no_label(block, &block_at);
                c.columns(&block.body, &block_at, &mut |c, b, path| {
                    if b.kind == "slot" {
                        slot(c, b, fields, &mut placed, path);
                    } else {
                        c.content_item(b, path);
                        check_field_refs(c, b, fields, path);
                    }
                });
                items.push(TabItem::Block(block.clone()));
            } else if CONTENT_KINDS.contains(&block.kind.as_str()) {
                let block_at = format!("{tab_at} > {}#{}", block.kind, n + 1);
                c.no_label(block, &block_at);
                c.content_block(block, &block_at);
                check_field_refs(c, block, fields, &block_at);
                items.push(TabItem::Block(block.clone()));
            } else {
                c.fail(
                    &format!("{tab_at} > {}", block.name()),
                    "is neither a slot nor a content block",
                );
            }
        }
        tabs.push(Tab {
            title,
            when,
            items,
            extra: unknown(&t.body, &tab_items, &[]),
        });
    }
    c.template = false;
    Some(Template {
        name,
        label: string(d, "label"),
        tabs,
        extra: unknown(d, &template_items, &[]),
    })
}

/// Fields every note has, which a tab's `when` may read besides the type's own.
const NOTE_BUILTINS: [&str; 6] = ["id", "type", "title", "created", "updated", "archived"];

/// A tab's `when`: an expression in the syntax of a query's `where` clause that reads only the
/// note shown, since the client evaluates it on that note alone. Every name is a field of the
/// type or a built-in, `.` reads only `this` or a date range, location or dict field, and
/// `linked_from()`, which reads other notes, is refused.
fn check_when(c: &mut Check, text: &str, fields: &[FieldDef], at: &str) {
    let expr = match query_syntax::parse_expr(text) {
        Ok(e) => e,
        Err(e) => {
            c.fail(at, format!("does not parse: {e}"));
            return;
        }
    };
    if let Err(message) = when_expr(&expr, fields, &mut Vec::new()) {
        c.fail(at, message);
    }
}

fn when_name(name: &str, fields: &[FieldDef]) -> Result<(), String> {
    if NOTE_BUILTINS.contains(&name) || fields.iter().any(|f| f.name == name) {
        Ok(())
    } else {
        Err(format!("`{name}` is not a field of the type"))
    }
}

fn when_expr(e: &Expr, fields: &[FieldDef], bound: &mut Vec<String>) -> Result<(), String> {
    match e {
        Expr::Lit(_) | Expr::This => Ok(()),
        Expr::Name(n) if bound.contains(n) => Ok(()),
        Expr::Name(n) => when_name(n, fields),
        Expr::Field(base, member) => {
            // `this.x` is the shown note's field `x`, as a bare `x` is.
            let name = match base.as_ref() {
                Expr::This => return when_name(member, fields),
                Expr::Name(n) if !bound.contains(n) => n,
                Expr::Field(inner, n) if matches!(inner.as_ref(), Expr::This) => n,
                _ => {
                    return Err(format!(
                        "`.{member}` reads another note, which a tab condition cannot"
                    ));
                }
            };
            when_name(name, fields)?;
            match fields.iter().find(|f| &f.name == name).map(|f| &f.kind) {
                Some(
                    FieldKind::DateRange { .. }
                    | FieldKind::Location { .. }
                    | FieldKind::Dict { .. },
                ) => Ok(()),
                Some(FieldKind::Reference(_)) => Err(format!(
                    "`{name}.{member}` reads another note, which a tab condition cannot"
                )),
                _ => Err(format!(
                    "`{name}` has no members: `.` reads a date range's start or end"
                )),
            }
        }
        Expr::Call(f, _) if f == "linked_from" => {
            Err("linked_from() reads other notes, which a tab condition cannot".to_owned())
        }
        Expr::Call(_, args) => args.iter().try_for_each(|a| when_expr(a, fields, bound)),
        Expr::Unary(_, inner) => when_expr(inner, fields, bound),
        Expr::Binary(_, a, b) => {
            when_expr(a, fields, bound)?;
            when_expr(b, fields, bound)
        }
        Expr::Closure(params, body) => {
            let before = bound.len();
            bound.extend(params.iter().cloned());
            let r = when_expr(body, fields, bound);
            bound.truncate(before);
            r
        }
    }
}

/// A `slot "<field>"`, in a tab or in a column of one: it names `title` or a field of the type,
/// and places that field once per template.
fn slot(
    c: &mut Check,
    block: &DocBlock,
    fields: &[FieldDef],
    placed: &mut HashSet<String>,
    at: &str,
) -> Option<TabItem> {
    let [field] = block.labels.as_slice() else {
        c.fail(at, "needs exactly one label, the field name");
        return None;
    };
    if field != "title" && !fields.iter().any(|f| &f.name == field) {
        c.fail(at, format!("the type has no field {field:?}"));
    } else if !placed.insert(field.clone()) {
        c.fail(
            at,
            format!("places {field:?} a second time in this template"),
        );
    }
    let slot_items = [opt("label", Form::Str)];
    c.attributes(&block.body, &slot_items, &[], at, "slots");
    c.only_blocks(&block.body, &[], at);
    Some(TabItem::Slot {
        field: field.clone(),
        label: string(&block.body, "label"),
        extra: unknown(&block.body, &slot_items, &[]),
    })
}

/// Every `{{name}}` in the `text` items of a fixed template block, an admonition's `title`, and an image's `alt` and
/// `caption`, at any depth, must name `title` or a field of the type. `{{name}}` is
/// template-only markup: in a note it is text.
fn check_field_refs(c: &mut Check, b: &DocBlock, fields: &[FieldDef], at: &str) {
    let items: &[&str] = if b.kind == "image" {
        &["text", "alt", "caption"]
    } else if b.kind == "admonition" {
        &["title"]
    } else {
        &["text"]
    };
    for item in items {
        let Some(text) = string(&b.body, item) else {
            continue;
        };
        for name in field_refs(&text) {
            if name != "title" && !fields.iter().any(|f| f.name == name) {
                c.fail(
                    &format!("{at}.{item}"),
                    format!("`{{{{{name}}}}}` names no field of the type"),
                );
            }
        }
    }
    for child in &b.body.blocks {
        check_field_refs(c, child, fields, &format!("{at} > {}", child.name()));
    }
}

/// The names of the `{{name}}` references in inline markup, in order. A backslash-escaped `{`
/// starts no reference, and a code span holds none. `name` is an identifier.
pub fn field_refs(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '`' => match chars[i + 1..].iter().position(|&c| c == '`') {
                Some(k) => i += k + 2,
                None => i += 1,
            },
            '{' if chars.get(i + 1) == Some(&'{') => {
                let start = i + 2;
                let mut j = start;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[start..j].iter().collect();
                if j > start
                    && !chars[start].is_ascii_digit()
                    && chars.get(j) == Some(&'}')
                    && chars.get(j + 1) == Some(&'}')
                {
                    out.push(name);
                    i = j + 2;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Check a `list_page` block: `view`, `sort` and `page_size`, each optional (lists L1).
fn list_page_of(c: &mut Check, b: &DocBlock, fields: &[FieldDef]) -> ListPage {
    let at = "list_page";
    c.no_label(b, at);
    let d = &b.body;
    let items = [
        opt("view", Form::OneOf(&LIST_PAGE_VIEWS)),
        opt("sort", Form::Str),
        opt(
            "page_size",
            Form::Int(LIST_PAGE_SIZE_MIN, LIST_PAGE_SIZE_MAX),
        ),
    ];
    c.items(d, &items, &[], at);
    c.only_blocks(d, &[], at);
    let sort = string(d, "sort").and_then(|text| {
        let words: Vec<&str> = text.split_whitespace().collect();
        let (name, desc) = match words.as_slice() {
            [name] => (*name, false),
            [name, "asc"] => (*name, false),
            [name, "desc"] => (*name, true),
            _ => {
                c.fail(
                    &format!("{at}.sort"),
                    format!("{text:?} must be a field name, then asc or desc, such as \"updated desc\""),
                );
                return None;
            }
        };
        let known = LIST_PAGE_SORT_BUILT_INS.contains(&name) || fields.iter().any(|f| f.name == name);
        if !known {
            c.fail(
                &format!("{at}.sort"),
                format!("names {name:?}, which is not a field of the type, nor title, created or updated"),
            );
            return None;
        }
        Some((name.to_owned(), desc))
    });
    ListPage {
        view: string(d, "view"),
        sort,
        page_size: d.field("page_size").and_then(|v| match v {
            Value::Int(i) => Some(*i),
            _ => None,
        }),
    }
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

/// The attributes of `doc` that neither `items` nor `also` name: what [`Check::attributes`] warns
/// this version ignores, in file order, first occurrence only.
fn unknown(doc: &Doc, items: &[crate::blocks::Item], also: &[&str]) -> Vec<(String, Value)> {
    let mut seen = HashSet::new();
    doc.fields
        .iter()
        .filter(|(n, _)| {
            !items.iter().any(|i| i.name == n) && !also.contains(&n.as_str()) && seen.insert(n)
        })
        .cloned()
        .collect()
}

pub(crate) fn string(doc: &Doc, name: &str) -> Option<String> {
    doc.field(name).and_then(Value::as_str).map(str::to_owned)
}
