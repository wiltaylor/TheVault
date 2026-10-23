//! Content blocks, ink, canvas and query views (D15, D29, D33, D34), as `docs/vault-format.md`
//! defines them. A content field, a template tab and a note's ink and canvas fields all hold these.
//!
//! Each block's items are a table of [`Item`]s. What a table cannot say, such as a canvas edge
//! naming a node of the same canvas, is checked in code after it.

use std::collections::HashSet;

use crate::Issue;
use crate::doc::{Doc, DocBlock, Value};
use crate::encoding::{PALETTE, is_ident, is_ink_colour, is_palette, is_ulid};
use crate::types::{FieldDef, FieldKind, RangeOf, Scalar};

/// The closed set of content block kinds.
pub const CONTENT_KINDS: [&str; 15] = [
    "heading",
    "paragraph",
    "list",
    "quote",
    "image",
    "asset",
    "code",
    "mermaid",
    "ink",
    "query_view",
    "canvas",
    "columns",
    "control",
    "toggle",
    "admonition",
];

/// The content block kinds a canvas card holds: every content block kind but a canvas.
pub const CARD_BLOCK_KINDS: [&str; 14] = [
    "heading",
    "paragraph",
    "list",
    "quote",
    "image",
    "asset",
    "code",
    "mermaid",
    "ink",
    "query_view",
    "columns",
    "control",
    "toggle",
    "admonition",
];

/// The kinds of an `admonition` block, in the order the editor offers them.
pub const ADMONITION_KINDS: [&str; 8] = [
    "note", "info", "tip", "success", "warning", "danger", "question", "quote",
];

/// What an admonition of one kind looks like when it names no `colour`, `icon` or `title`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmonitionPreset {
    /// A palette colour (OQ-S4).
    pub colour: &'static str,
    /// An icon name, as [`is_icon`] knows them.
    pub icon: &'static str,
    /// The title shown when the block has none.
    pub label: &'static str,
}

/// The preset of admonition `kind`, or `None` for a kind outside [`ADMONITION_KINDS`].
pub fn admonition_preset(kind: &str) -> Option<AdmonitionPreset> {
    let (colour, icon, label) = match kind {
        "note" => ("gray", "pencil", "Note"),
        "info" => ("blue", "info", "Info"),
        "tip" => ("green", "lightbulb", "Tip"),
        "success" => ("green", "circle-check", "Success"),
        "warning" => ("orange", "triangle-alert", "Warning"),
        "danger" => ("red", "octagon-alert", "Danger"),
        "question" => ("purple", "circle-question-mark", "Question"),
        "quote" => ("brown", "quote", "Quote"),
        _ => return None,
    };
    Some(AdmonitionPreset {
        colour,
        icon,
        label,
    })
}

/// The icon names the web client draws, one per line and sorted: Lucide's names at the version
/// `web/package.json` pins, in the client's kebab-case form. A web test keeps the two the same.
const ICONS: &str = include_str!("icons.txt");

/// Whether `name` is an icon the client can draw, such as `triangle-alert`.
pub fn is_icon(name: &str) -> bool {
    ICONS.lines().any(|l| l == name)
}

/// The kinds of a `control` block: the form controls a note field can be edited with.
pub const CONTROL_KINDS: [&str; 13] = [
    "text", "textarea", "number", "slider", "toggle", "checkbox", "dropdown", "radio", "chips",
    "calendar", "datetime", "time", "rating",
];

/// A rating control's `max` when it has none, and the largest it may have.
pub const RATING_MAX_DEFAULT: i64 = 5;
pub const RATING_MAX_LIMIT: i64 = 10;

/// The fields a control of `kind` fits, in words, for diagnostics.
pub fn control_fits_text(kind: &str) -> &'static str {
    match kind {
        "text" => "a string, url, int or float field",
        "textarea" => "a string field",
        "number" | "slider" => "an int or float field",
        "toggle" | "checkbox" => "a bool field",
        "dropdown" => "a single select, taxonomy or reference field",
        "radio" => "a single select field",
        "chips" => "a list of strings, or a multiple select, taxonomy or reference field",
        "calendar" => "a date field or a date range of dates",
        "datetime" | "time" => "a datetime field",
        "rating" => "an int field",
        _ => "no field",
    }
}

/// Whether a control of `kind` can show and edit field `f` (docs/vault-format.md, "control").
pub fn control_fits(kind: &str, f: &FieldDef) -> bool {
    use FieldKind as K;
    let choice = matches!(
        f.kind,
        K::Select { .. } | K::Taxonomy { .. } | K::Reference(_)
    );
    match kind {
        "text" => matches!(f.kind, K::String | K::Url | K::Int | K::Float),
        "textarea" => matches!(f.kind, K::String),
        "number" | "slider" => matches!(f.kind, K::Int | K::Float),
        "toggle" | "checkbox" => matches!(f.kind, K::Bool),
        "dropdown" => choice && !f.multiple,
        "radio" => matches!(f.kind, K::Select { .. }) && !f.multiple,
        "chips" => {
            (choice && f.multiple)
                || matches!(
                    f.kind,
                    K::List {
                        items: Scalar::String
                    }
                )
        }
        "calendar" => matches!(f.kind, K::Date | K::DateRange { of: RangeOf::Date }),
        "datetime" | "time" => matches!(f.kind, K::Datetime),
        "rating" => matches!(f.kind, K::Int),
        _ => false,
    }
}

/// A field's kind in words: `a multiple select field`, `a list of int field`.
fn field_kind_text(f: &FieldDef) -> String {
    let what = match &f.kind {
        FieldKind::List { items } => format!("list of {} ", scalar_name(*items)),
        _ if f.term_list => "list of taxonomy terms ".to_owned(),
        FieldKind::DateRange {
            of: RangeOf::Datetime,
        } => "date range of datetimes ".to_owned(),
        FieldKind::DateRange {
            of: RangeOf::DateOrDatetime,
        } => "date range of dates or datetimes ".to_owned(),
        k if f.multiple => format!("multiple {} ", k.name()),
        k => format!("{} ", k.name()),
    };
    let article = if what.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {what}field")
}

fn scalar_name(s: Scalar) -> &'static str {
    match s {
        Scalar::String => "string",
        Scalar::Int => "int",
        Scalar::Float => "float",
        Scalar::Bool => "bool",
        Scalar::Date => "date",
        Scalar::Datetime => "datetime",
        Scalar::Url => "url",
    }
}

/// How many `column` blocks a `columns` block holds, at least and at most.
pub const COLUMNS_MIN: usize = 2;
pub const COLUMNS_MAX: usize = 6;

/// The kinds of a canvas node. Groups are blocks of their own, not nodes.
pub const NODE_KINDS: [&str; 2] = ["card", "note"];

/// Where an edge draws its arrowheads: `start` is the end at the `from` node.
pub const ARROWS: [&str; 4] = ["none", "start", "end", "both"];

/// A canvas's `height`, in screen pixels: the default when it has none, and the bounds.
pub const CANVAS_HEIGHT_DEFAULT: i64 = 480;
pub const CANVAS_HEIGHT_MIN: i64 = 160;
pub const CANVAS_HEIGHT_MAX: i64 = 4000;

/// A `map` view's `height`, in screen pixels: the default when it has none, and the bounds.
pub const MAP_VIEW_HEIGHT_DEFAULT: i64 = 448;
pub const MAP_VIEW_HEIGHT_MIN: i64 = 160;
pub const MAP_VIEW_HEIGHT_MAX: i64 = 4000;

/// An `asset` block's `height`, the height of its PDF viewer in screen pixels: the bounds.
pub const ASSET_HEIGHT_MIN: i64 = 160;
pub const ASSET_HEIGHT_MAX: i64 = 4000;

/// An ink drawing's `height`, in ink units: the default when it has none, and the bounds. The
/// minimum stays 1 so drawings made before the resize handle stay valid; the handle itself keeps
/// to 80 and up.
pub const INK_HEIGHT_DEFAULT: i64 = 400;
pub const INK_HEIGHT_MIN: i64 = 1;
pub const INK_HEIGHT_MAX: i64 = 4000;

/// The kinds of a `query_view`'s `view`.
pub const VIEW_KINDS: [&str; 9] = [
    "table", "cards", "list", "details", "kanban", "calendar", "timeline", "map", "canvas",
];

/// What one item's value must be.
#[derive(Clone, Copy)]
pub(crate) enum Form {
    Str,
    Bool,
    /// An integer in an inclusive range.
    Int(i64, i64),
    /// Any number.
    Num,
    /// A number greater than zero.
    Positive,
    Ulid,
    Palette,
    Icon,
    InkColour,
    Ident,
    StrList,
    OneOf(&'static [&'static str]),
}

pub(crate) struct Item {
    pub name: &'static str,
    pub form: Form,
    pub required: bool,
}

pub(crate) const fn req(name: &'static str, form: Form) -> Item {
    Item {
        name,
        form,
        required: true,
    }
}

pub(crate) const fn opt(name: &'static str, form: Form) -> Item {
    Item {
        name,
        form,
        required: false,
    }
}

/// Collects issues under a path.
pub(crate) struct Check<'a> {
    pub issues: &'a mut Vec<Issue>,
    /// The fields of the type whose content is checked, when known: a `control` block must
    /// name one of them that it fits.
    pub fields: Option<&'a [FieldDef]>,
    /// Whether the blocks checked are a template's fixed blocks, where an `image` may show an
    /// asset field of the note (`field`) in place of a fixed `asset`.
    pub template: bool,
}

impl Check<'_> {
    pub fn fail(&mut self, at: &str, message: impl Into<String>) {
        self.issues.push(Issue::new(at, message));
    }

    /// Report something this version ignores, which leaves the file valid.
    pub fn warn(&mut self, at: &str, message: impl Into<String>) {
        self.issues.push(Issue::warning(at, message));
    }

    /// How many errors have been found, leaving out warnings.
    pub fn errors(&self) -> usize {
        self.issues.iter().filter(|i| !i.is_warning()).count()
    }

    /// Check `doc`'s fields against `items`: no unknown or repeated item, every required one
    /// present, every value of its form. `also` names further fields the caller checks itself.
    pub fn items(&mut self, doc: &Doc, items: &[Item], also: &[&str], at: &str) {
        self.check_items(doc, items, also, at, None);
    }

    /// [`Check::items`] for the attributes of a type or a field, where an unknown one is a warning
    /// rather than an error: a newer release may have added it, and an older reader still loads
    /// the definition, ignoring it. `owner` names what the attributes belong to, as "int fields".
    pub fn attributes(&mut self, doc: &Doc, items: &[Item], also: &[&str], at: &str, owner: &str) {
        self.check_items(doc, items, also, at, Some(owner));
    }

    fn check_items(
        &mut self,
        doc: &Doc,
        items: &[Item],
        also: &[&str],
        at: &str,
        tolerant: Option<&str>,
    ) {
        let mut seen = HashSet::new();
        for (name, value) in &doc.fields {
            let path = format!("{at}.{name}");
            if !seen.insert(name.as_str()) {
                self.fail(&path, "appears more than once");
                continue;
            }
            match items.iter().find(|i| i.name == name) {
                Some(item) => self.form(value, item.form, &path),
                None if also.contains(&name.as_str()) => {}
                // Inside a template, every block's attributes are tolerated as a type's are, so
                // a template written by a newer release still loads.
                None => match tolerant.or(self.template.then_some("template blocks")) {
                    Some(owner) => self.warn(
                        &path,
                        format!("is not an attribute of {owner} in this version; it is ignored"),
                    ),
                    None => self.fail(&path, "is not an item of this block"),
                },
            }
        }
        for item in items.iter().filter(|i| i.required) {
            if doc.field(item.name).is_none() {
                self.fail(&format!("{at}.{}", item.name), "is required");
            }
        }
    }

    pub fn form(&mut self, value: &Value, form: Form, at: &str) {
        let problem = match (form, value) {
            (Form::Str, Value::Str(_)) | (Form::Bool, Value::Bool(_)) => None,
            (Form::Int(lo, hi), Value::Int(i)) if (lo..=hi).contains(i) => None,
            (Form::Int(lo, hi), Value::Int(_)) => Some(format!("must be from {lo} to {hi}")),
            (Form::Num, v) if v.as_f64().is_some() => None,
            (Form::Positive, v) if v.as_f64().is_some_and(|f| f > 0.0) => None,
            (Form::Positive, v) if v.as_f64().is_some() => Some("must be greater than 0".into()),
            (Form::Ulid, Value::Str(s)) if is_ulid(s) => None,
            (Form::Ulid, Value::Str(s)) => Some(format!("{s:?} is not a ULID")),
            (Form::Palette, Value::Str(s)) if is_palette(s) => None,
            (Form::Palette, Value::Str(s)) => Some(format!(
                "{s:?} is not a palette colour; use one of {}",
                PALETTE.join(", ")
            )),
            (Form::Icon, Value::Str(s)) if is_icon(s) => None,
            (Form::Icon, Value::Str(s)) => Some(format!(
                "{s:?} is not an icon name; use a Lucide icon name such as \"triangle-alert\""
            )),
            (Form::InkColour, Value::Str(s)) if is_ink_colour(s) => None,
            (Form::InkColour, Value::Str(s)) => Some(format!(
                "{s:?} is not a lower-case #rrggbb colour or \"ink\""
            )),
            (Form::Ident, Value::Str(s)) if is_ident(s) => None,
            (Form::Ident, Value::Str(s)) => Some(format!("{s:?} is not an identifier")),
            (Form::OneOf(set), Value::Str(s)) if set.contains(&s.as_str()) => None,
            (Form::OneOf(set), Value::Str(s)) => {
                Some(format!("{s:?} is not one of {}", set.join(", ")))
            }
            (Form::StrList, Value::List(l)) if l.iter().all(|v| v.as_str().is_some()) => None,
            (Form::StrList, _) => Some(format!(
                "must be a list of strings, not {}",
                value.describe()
            )),
            (Form::Str | Form::Ulid | Form::Palette | Form::Icon, _)
            | (Form::InkColour | Form::Ident | Form::OneOf(_), _) => {
                Some(format!("must be a string, not {}", value.describe()))
            }
            (Form::Bool, _) => Some(format!("must be true or false, not {}", value.describe())),
            (Form::Int(..), _) => Some(format!("must be an integer, not {}", value.describe())),
            (Form::Num | Form::Positive, _) => {
                Some(format!("must be a number, not {}", value.describe()))
            }
        };
        if let Some(p) = problem {
            self.fail(at, p);
        }
    }

    /// Reject any block in `doc` whose kind is not in `kinds`.
    pub fn only_blocks(&mut self, doc: &Doc, kinds: &[&str], at: &str) {
        for b in &doc.blocks {
            if !kinds.contains(&b.kind.as_str()) {
                self.fail(&format!("{at} > {}", b.name()), "is not allowed here");
            }
        }
    }

    pub fn no_label(&mut self, b: &DocBlock, at: &str) {
        if !b.labels.is_empty() {
            self.fail(at, "takes no label");
        }
    }

    /// A block labelled with one identifier.
    pub fn ident_label<'b>(&mut self, b: &'b DocBlock, at: &str) -> Option<&'b str> {
        match b.labels.as_slice() {
            [l] if is_ident(l) => Some(l),
            [l] => {
                self.fail(at, format!("label {l:?} is not an identifier"));
                None
            }
            _ => {
                self.fail(at, "needs exactly one label");
                None
            }
        }
    }

    /// Every block of a content sequence: a content field's body, or the content blocks of a tab.
    pub fn content_blocks<'b>(&mut self, blocks: impl IntoIterator<Item = &'b DocBlock>, at: &str) {
        for (n, b) in blocks.into_iter().enumerate() {
            let path = format!("{at} > {}#{}", b.kind, n + 1);
            self.content_item(b, &path);
        }
    }

    /// One block of a content sequence, at `at`: a content block kind, unlabelled, and valid.
    pub fn content_item(&mut self, b: &DocBlock, at: &str) {
        if !CONTENT_KINDS.contains(&b.kind.as_str()) {
            self.fail(at, format!("{:?} is not a content block kind", b.kind));
            return;
        }
        self.no_label(b, at);
        self.content_block(b, at);
    }

    /// The body of a `columns` block: 2 to 6 `column` blocks, each with an optional positive
    /// `width` and an ordered sequence of blocks. `child` checks each block of a column; a
    /// column never holds another `columns` block. In a template, `child` also takes slots.
    pub fn columns(
        &mut self,
        d: &Doc,
        at: &str,
        child: &mut dyn FnMut(&mut Self, &DocBlock, &str),
    ) {
        self.items(d, &[], &[], at);
        self.only_blocks(d, &["column"], at);
        let count = d.blocks_of("column").count();
        if !(COLUMNS_MIN..=COLUMNS_MAX).contains(&count) {
            self.fail(
                at,
                format!("needs {COLUMNS_MIN} to {COLUMNS_MAX} columns, not {count}"),
            );
        }
        for (n, col) in d.blocks_of("column").enumerate() {
            let path = format!("{at} > column#{}", n + 1);
            self.no_label(col, &path);
            self.items(&col.body, &[opt("width", Form::Positive)], &[], &path);
            for (k, b) in col.body.blocks.iter().enumerate() {
                let block_at = format!("{path} > {}#{}", b.kind, k + 1);
                if b.kind == "columns" {
                    self.fail(&block_at, "a column cannot hold another columns block");
                    continue;
                }
                child(self, b, &block_at);
            }
        }
    }

    /// The blocks nested in a toggle or an admonition (`what`): any content block but a column
    /// layout, which stays at the top of a content sequence or a card, as a column holds none.
    fn nested(&mut self, d: &Doc, what: &str, at: &str) {
        for (n, b) in d.blocks.iter().enumerate() {
            let path = format!("{at} > {}#{}", b.kind, n + 1);
            if b.kind == "columns" {
                self.fail(&path, format!("{what} cannot hold a columns block"));
                continue;
            }
            self.content_item(b, &path);
        }
    }

    /// One content block's body. The caller has checked its kind and label.
    pub fn content_block(&mut self, b: &DocBlock, at: &str) {
        let d = &b.body;
        match b.kind.as_str() {
            "heading" => {
                self.items(
                    d,
                    &[req("level", Form::Int(1, 3)), req("text", Form::Str)],
                    &[],
                    at,
                );
                self.only_blocks(d, &[], at);
            }
            "paragraph" | "quote" => {
                self.items(d, &[req("text", Form::Str)], &[], at);
                self.only_blocks(d, &[], at);
            }
            "list" => self.list(d, at),
            "image" => self.image(d, at),
            "asset" => {
                self.items(
                    d,
                    &[
                        req("asset", Form::Ulid),
                        opt("caption", Form::Str),
                        opt("height", Form::Int(ASSET_HEIGHT_MIN, ASSET_HEIGHT_MAX)),
                        opt("card", Form::Bool),
                    ],
                    &[],
                    at,
                );
                self.only_blocks(d, &[], at);
            }
            "code" => {
                let items = [
                    opt("language", Form::Str),
                    opt("caption", Form::Str),
                    req("source", Form::Str),
                ];
                self.items(d, &items, &[], at);
                self.only_blocks(d, &[], at);
            }
            "mermaid" => {
                self.items(d, &[req("source", Form::Str)], &[], at);
                self.only_blocks(d, &[], at);
            }
            "ink" => self.ink(d, at),
            "query_view" => self.query_view(d, at),
            "canvas" => self.canvas(d, at),
            "columns" => self.columns(d, at, &mut |c, b, path| c.content_item(b, path)),
            "control" => self.control(d, at),
            "toggle" => {
                self.items(
                    d,
                    &[opt("level", Form::Int(0, 3)), req("text", Form::Str)],
                    &[],
                    at,
                );
                self.nested(d, "a toggle", at);
            }
            "admonition" => {
                let items = [
                    req("kind", Form::OneOf(&ADMONITION_KINDS)),
                    opt("title", Form::Str),
                    opt("colour", Form::Palette),
                    opt("icon", Form::Icon),
                    opt("collapsible", Form::Bool),
                ];
                self.items(d, &items, &[], at);
                self.nested(d, "an admonition", at);
            }
            _ => {}
        }
    }

    /// An `image` block: a fixed `asset`, or in a template the asset `field` of the note shown.
    /// Exactly one of the two.
    fn image(&mut self, d: &Doc, at: &str) {
        let items = [
            opt("asset", Form::Ulid),
            opt("field", Form::Str),
            opt("alt", Form::Str),
            opt("caption", Form::Str),
            opt("width", Form::Int(1, i64::MAX)),
        ];
        self.items(d, &items, &[], at);
        self.only_blocks(d, &[], at);
        let field = d.field("field");
        if field.is_some() && !self.template {
            return self.fail(
                &format!("{at}.field"),
                "is allowed only in a template; a note's image names its `asset`",
            );
        }
        match (d.field("asset"), field) {
            (Some(_), Some(_)) => {
                return self.fail(at, "takes `asset` or `field`, not both");
            }
            (None, None) if self.template => {
                return self.fail(at, "needs `asset` or `field`");
            }
            (None, None) => return self.fail(&format!("{at}.asset"), "is required"),
            _ => {}
        }
        let (Some(fields), Some(Value::Str(name))) = (self.fields, field) else {
            return;
        };
        match fields.iter().find(|f| &f.name == name) {
            None => self.fail(
                &format!("{at}.field"),
                format!("{name:?} names no field of the type"),
            ),
            Some(f) if !matches!(f.kind, FieldKind::Asset) => self.fail(
                &format!("{at}.field"),
                format!(
                    "an image shows an asset field, and {name:?} is {}",
                    field_kind_text(f)
                ),
            ),
            Some(_) => {}
        }
    }

    /// A `control` block: a form control bound to a field of the note. Its items are checked on
    /// their own; with the type's fields known, the field must exist and the control fit it.
    fn control(&mut self, d: &Doc, at: &str) {
        let items = [
            req("field", Form::Str),
            req("kind", Form::OneOf(&CONTROL_KINDS)),
            opt("label", Form::Str),
            opt("placeholder", Form::Str),
            opt("min", Form::Num),
            opt("max", Form::Num),
            opt("step", Form::Positive),
        ];
        self.items(d, &items, &[], at);
        self.only_blocks(d, &[], at);
        let Some(Value::Str(kind)) = d.field("kind") else {
            return;
        };
        let kind = kind.as_str();
        if !CONTROL_KINDS.contains(&kind) {
            return;
        }
        // The items each control kind uses beyond field, kind and label, and those it needs.
        let (uses, needs): (&[&str], &[&str]) = match kind {
            "text" | "textarea" | "dropdown" | "chips" => (&["placeholder"], &[]),
            "number" => (&["placeholder", "min", "max", "step"], &[]),
            "slider" => (&["min", "max", "step"], &["min", "max"]),
            "rating" => (&["max"], &[]),
            _ => (&[], &[]),
        };
        for (name, _) in &d.fields {
            if !matches!(name.as_str(), "field" | "kind" | "label")
                && !uses.contains(&name.as_str())
            {
                self.fail(
                    &format!("{at}.{name}"),
                    format!("is not used by a {kind} control"),
                );
            }
        }
        for n in needs {
            if d.field(n).is_none() {
                self.fail(
                    &format!("{at}.{n}"),
                    format!("is required by a {kind} control"),
                );
            }
        }
        if kind == "rating" {
            match d.field("max") {
                None | Some(Value::Int(1..=RATING_MAX_LIMIT)) => {}
                Some(Value::Int(_)) => self.fail(
                    &format!("{at}.max"),
                    format!("must be from 1 to {RATING_MAX_LIMIT} stars"),
                ),
                Some(_) => self.fail(&format!("{at}.max"), "must be a whole number of stars"),
            }
        } else if let (Some(lo), Some(hi)) = (
            d.field("min").and_then(Value::as_f64),
            d.field("max").and_then(Value::as_f64),
        ) && lo >= hi
        {
            self.fail(&format!("{at}.max"), "must be greater than min");
        }

        let Some(fields) = self.fields else { return };
        let Some(Value::Str(name)) = d.field("field") else {
            return;
        };
        let title;
        let f = if name == "title" {
            title = crate::types::title_field();
            Some(&title)
        } else {
            fields.iter().find(|f| &f.name == name)
        };
        let Some(f) = f else {
            return self.fail(
                &format!("{at}.field"),
                format!("the type has no field {name:?}"),
            );
        };
        if !control_fits(kind, f) {
            return self.fail(
                &format!("{at}.field"),
                format!(
                    "a {kind} control fits {}, and {name:?} is {}",
                    control_fits_text(kind),
                    field_kind_text(f)
                ),
            );
        }
        if matches!(f.kind, FieldKind::Int) {
            for n in ["min", "max", "step"] {
                if kind != "rating"
                    && let Some(Value::Float(_)) = d.field(n)
                {
                    self.fail(
                        &format!("{at}.{n}"),
                        format!("must be a whole number, as {name:?} is an int field"),
                    );
                }
            }
        }
    }

    fn list(&mut self, d: &Doc, at: &str) {
        const STYLES: &[&str] = &["bullet", "numbered", "todo"];
        self.items(d, &[req("style", Form::OneOf(STYLES))], &[], at);
        self.only_blocks(d, &["item"], at);
        if d.blocks_of("item").next().is_none() {
            self.fail(at, "needs at least one item");
        }
        let todo = d.field("style").and_then(Value::as_str) == Some("todo");
        self.list_items(d, todo, at);
    }

    fn list_items(&mut self, d: &Doc, todo: bool, at: &str) {
        for (n, item) in d.blocks_of("item").enumerate() {
            let path = format!("{at} > item#{}", n + 1);
            self.no_label(item, &path);
            self.items(
                &item.body,
                &[req("text", Form::Str), opt("checked", Form::Bool)],
                &[],
                &path,
            );
            if !todo && item.body.field("checked").is_some() {
                self.fail(&format!("{path}.checked"), "is allowed only in todo lists");
            }
            self.only_blocks(&item.body, &["item"], &path);
            self.list_items(&item.body, todo, &path);
        }
    }

    /// The body of an ink block, whether a content block or a note's ink field.
    pub fn ink(&mut self, d: &Doc, at: &str) {
        const BACKGROUNDS: &[&str] = &["blank", "lines", "grid", "dots"];
        let items = [
            opt("width", Form::Int(1, i64::MAX)),
            opt("height", Form::Int(INK_HEIGHT_MIN, INK_HEIGHT_MAX)),
            opt("background", Form::OneOf(BACKGROUNDS)),
        ];
        self.items(d, &items, &[], at);
        self.only_blocks(d, &["viewport", "stroke"], at);
        if d.blocks_of("viewport").count() > 1 {
            self.fail(at, "has more than one viewport");
        }
        for v in d.blocks_of("viewport") {
            let path = format!("{at} > viewport");
            self.no_label(v, &path);
            let items = [
                req("x", Form::Num),
                req("y", Form::Num),
                req("zoom", Form::Positive),
            ];
            self.items(&v.body, &items, &[], &path);
            self.only_blocks(&v.body, &[], &path);
        }
        self.strokes(d, at);
    }

    fn strokes(&mut self, d: &Doc, at: &str) {
        for (n, s) in d.blocks_of("stroke").enumerate() {
            let path = format!("{at} > stroke#{}", n + 1);
            self.no_label(s, &path);
            let items = [
                req("tool", Form::OneOf(&["pen", "highlighter"])),
                req("colour", Form::InkColour),
                req("size", Form::Positive),
            ];
            self.items(&s.body, &items, &["points"], &path);
            self.only_blocks(&s.body, &[], &path);
            match s.body.field("points") {
                None => self.fail(&format!("{path}.points"), "is required"),
                Some(v) => self.points(v, &format!("{path}.points")),
            }
        }
    }

    fn points(&mut self, v: &Value, at: &str) {
        let Value::List(l) = v else {
            return self.fail(
                at,
                format!("must be a list of numbers, not {}", v.describe()),
            );
        };
        let nums: Option<Vec<f64>> = l.iter().map(Value::as_f64).collect();
        let Some(nums) = nums else {
            return self.fail(at, "must be a list of numbers");
        };
        if nums.is_empty() || nums.len() % 3 != 0 {
            return self.fail(at, "must hold at least one x, y, pressure triple");
        }
        if nums.chunks(3).any(|t| !(0.0..=1.0).contains(&t[2])) {
            self.fail(at, "pressure must be from 0.0 to 1.0");
        }
    }

    fn query_view(&mut self, d: &Doc, at: &str) {
        let items = [
            opt("title", Form::Str),
            req("query", Form::Str),
            opt("default_view", Form::Ident),
        ];
        self.items(d, &items, &[], at);
        self.only_blocks(d, &["view"], at);
        let mut names = HashSet::new();
        for v in d.blocks_of("view") {
            let path = format!("{at} > {}", v.name());
            if let Some(name) = self.ident_label(v, &path)
                && !names.insert(name)
            {
                self.fail(&path, "names a view twice");
            }
            self.view(&v.body, &path);
        }
        if d.blocks_of("view").next().is_none() {
            self.fail(at, "needs at least one view");
        }
        if let Some(Value::Str(dv)) = d.field("default_view")
            && !names.contains(dv.as_str())
        {
            self.fail(
                &format!("{at}.default_view"),
                format!("names no view {dv:?}"),
            );
        }
    }

    fn view(&mut self, d: &Doc, at: &str) {
        let items = [
            req("kind", Form::OneOf(&VIEW_KINDS)),
            opt("title", Form::Str),
            opt("template", Form::OneOf(&crate::types::TEMPLATE_NAMES)),
            opt("columns", Form::StrList),
            opt("group", Form::Str),
            opt("date", Form::Str),
            opt("mode", Form::OneOf(&["month", "week", "day"])),
            opt("location", Form::Str),
            opt(
                "height",
                Form::Int(MAP_VIEW_HEIGHT_MIN, MAP_VIEW_HEIGHT_MAX),
            ),
        ];
        self.items(d, &items, &[], at);
        let Some(Value::Str(kind)) = d.field("kind") else {
            return;
        };
        // The items each view kind uses, and the one it requires.
        let (uses, needs): (&[&str], Option<&str>) = match kind.as_str() {
            "table" => (&["columns"], None),
            "cards" | "details" => (&["template"], None),
            "list" => (&[], None),
            "kanban" => (&["template", "group"], Some("group")),
            "calendar" => (&["date", "mode"], Some("date")),
            "timeline" => (&["date"], Some("date")),
            "map" => (&["template", "location", "height"], Some("location")),
            "canvas" => (&["template"], None),
            _ => return,
        };
        for (name, _) in &d.fields {
            if !matches!(name.as_str(), "kind" | "title") && !uses.contains(&name.as_str()) {
                self.fail(
                    &format!("{at}.{name}"),
                    format!("is not used by a {kind} view"),
                );
            }
        }
        if let Some(n) = needs
            && d.field(n).is_none()
        {
            self.fail(
                &format!("{at}.{n}"),
                format!("is required by a {kind} view"),
            );
        }
        if kind == "canvas" {
            self.only_blocks(d, &["place"], at);
            for p in d.blocks_of("place") {
                let path = format!("{at} > {}", p.name());
                match p.labels.as_slice() {
                    [l] if is_ulid(l) => {}
                    _ => self.fail(&path, "needs one label, a note ULID"),
                }
                let items = [
                    req("x", Form::Num),
                    req("y", Form::Num),
                    opt("width", Form::Positive),
                    opt("height", Form::Positive),
                ];
                self.items(&p.body, &items, &[], &path);
                self.only_blocks(&p.body, &[], &path);
            }
        } else {
            self.only_blocks(d, &[], at);
        }
    }

    /// The body of a canvas block, whether a content block or a note's canvas field.
    pub fn canvas(&mut self, d: &Doc, at: &str) {
        self.items(
            d,
            &[opt(
                "height",
                Form::Int(CANVAS_HEIGHT_MIN, CANVAS_HEIGHT_MAX),
            )],
            &[],
            at,
        );
        self.only_blocks(d, &["viewport", "group", "node", "edge"], at);
        match d.blocks_of("viewport").count() {
            1 => {}
            0 => self.fail(at, "needs a viewport"),
            _ => self.fail(at, "has more than one viewport"),
        }
        for v in d.blocks_of("viewport") {
            let path = format!("{at} > viewport");
            self.no_label(v, &path);
            let items = [
                req("x", Form::Num),
                req("y", Form::Num),
                req("zoom", Form::Positive),
            ];
            self.items(&v.body, &items, &[], &path);
            self.only_blocks(&v.body, &[], &path);
        }

        let mut ids = HashSet::new();
        let mut groups = HashSet::new();
        let mut nodes = HashSet::new();
        for b in d.blocks.iter().filter(|b| b.kind != "viewport") {
            let path = format!("{at} > {}", b.name());
            let Some(id) = self.ident_label(b, &path) else {
                continue;
            };
            if !ids.insert(id) {
                self.fail(&path, "reuses an id already given to a group, node or edge");
            }
            match b.kind.as_str() {
                "group" => {
                    groups.insert(id);
                }
                "node" => {
                    nodes.insert(id);
                }
                _ => {}
            }
        }

        let frame = [
            req("x", Form::Num),
            req("y", Form::Num),
            req("width", Form::Positive),
            req("height", Form::Positive),
        ];
        for g in d.blocks_of("group") {
            let path = format!("{at} > {}", g.name());
            let mut items = vec![opt("label", Form::Str), opt("colour", Form::Palette)];
            items.extend(frame.iter().map(|i| req(i.name, i.form)));
            self.items(&g.body, &items, &[], &path);
            self.only_blocks(&g.body, &[], &path);
        }
        for n in d.blocks_of("node") {
            let path = format!("{at} > {}", n.name());
            self.node(&n.body, &frame, &groups, &path);
        }
        for e in d.blocks_of("edge") {
            let path = format!("{at} > {}", e.name());
            const SIDES: &[&str] = &["top", "right", "bottom", "left"];
            let items = [
                req("from", Form::Str),
                req("to", Form::Str),
                opt("from_side", Form::OneOf(SIDES)),
                opt("to_side", Form::OneOf(SIDES)),
                opt("label", Form::Str),
                opt("colour", Form::Palette),
                opt("arrow", Form::OneOf(&ARROWS)),
            ];
            self.items(&e.body, &items, &[], &path);
            self.only_blocks(&e.body, &[], &path);
            for end in ["from", "to"] {
                if let Some(Value::Str(id)) = e.body.field(end)
                    && !nodes.contains(id.as_str())
                {
                    self.fail(&format!("{path}.{end}"), format!("names no node {id:?}"));
                }
            }
        }
    }

    /// Fail every canvas nested anywhere inside `b`, a block of a card at `at`.
    fn no_canvas_within(&mut self, b: &DocBlock, at: &str) {
        let inner: Vec<(String, &DocBlock)> = if b.kind == "columns" {
            b.body
                .blocks_of("column")
                .enumerate()
                .flat_map(|(k, col)| {
                    col.body.blocks.iter().enumerate().map(move |(j, cb)| {
                        (
                            format!("{at} > column#{} > {}#{}", k + 1, cb.kind, j + 1),
                            cb,
                        )
                    })
                })
                .collect()
        } else if matches!(b.kind.as_str(), "toggle" | "admonition") {
            b.body
                .blocks
                .iter()
                .enumerate()
                .map(|(j, cb)| (format!("{at} > {}#{}", cb.kind, j + 1), cb))
                .collect()
        } else {
            return;
        };
        for (path, cb) in inner {
            if cb.kind == "canvas" {
                self.fail(&path, "a card cannot hold a canvas");
            }
            self.no_canvas_within(cb, &path);
        }
    }

    fn node(&mut self, d: &Doc, frame: &[Item], groups: &HashSet<&str>, at: &str) {
        let mut items = vec![
            req("kind", Form::OneOf(&NODE_KINDS)),
            opt("group", Form::Str),
            opt("colour", Form::Palette),
        ];
        items.extend(frame.iter().map(|i| req(i.name, i.form)));
        let kind = d.field("kind").and_then(Value::as_str).unwrap_or("");
        if kind == "note" {
            items.push(req("note", Form::Ulid));
            items.push(opt("template", Form::OneOf(&crate::types::TEMPLATE_NAMES)));
        }
        self.items(d, &items, &[], at);
        if kind == "card" {
            for (n, b) in d.blocks.iter().enumerate() {
                let path = format!("{at} > {}#{}", b.kind, n + 1);
                if b.kind == "canvas" {
                    self.fail(&path, "a card cannot hold a canvas");
                }
                // Nor can a column, a toggle or an admonition inside it, at any depth.
                self.no_canvas_within(b, &path);
            }
            let blocks = d.blocks.iter().filter(|b| b.kind != "canvas");
            self.content_blocks(blocks, at);
        } else {
            self.only_blocks(d, &[], at);
        }
        if let Some(Value::Str(g)) = d.field("group")
            && !groups.contains(g.as_str())
        {
            self.fail(&format!("{at}.group"), format!("names no group {g:?}"));
        }
    }
}
