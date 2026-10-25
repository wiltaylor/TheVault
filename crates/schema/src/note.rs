//! Note validation: a note's fields and blocks against its type, its references against the notes
//! that exist, and its terms against the taxonomies (D3, D4, D7, D23).

use std::collections::HashSet;

use crate::blocks::Check;
use crate::doc::{Doc, Value};
use crate::encoding::{
    is_date, is_datetime, is_time, is_ulid, is_url, is_utc_datetime, parse_date, parse_datetime,
};
use crate::types::{Accept, DictValues, FieldDef, FieldKind, RangeOf, Scalar, is_image_filename};
use crate::{Issue, Schema};

/// What validation needs to know about the rest of the vault.
pub trait NoteLookup {
    /// The type of the note with this id, or `None` when no such note exists.
    fn note_type(&self, id: &str) -> Option<String>;

    /// Whether `assets/<id>/` exists.
    fn asset_exists(&self, id: &str) -> bool;

    /// The file name of asset `id`, when it is known. An asset field that takes only images
    /// refuses an asset whose file name is known and is not an image's.
    fn asset_filename(&self, id: &str) -> Option<String> {
        let _ = id;
        None
    }
}

/// A note that passed validation. `unknown` names the items its type does not describe; the data
/// is kept and the note is flagged (D6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteReport {
    pub unknown: Vec<String>,
}

impl NoteReport {
    pub fn is_flagged(&self) -> bool {
        !self.unknown.is_empty()
    }
}

/// The vault around a note, for the checks that look beyond it.
pub(crate) struct Context<'a> {
    pub schema: &'a Schema,
    pub lookup: &'a dyn NoteLookup,
}

pub(crate) fn validate(
    schema: &Schema,
    type_name: &str,
    doc: &Doc,
    lookup: &dyn NoteLookup,
) -> Result<NoteReport, Vec<Issue>> {
    let mut issues = Vec::new();
    let Some(ty) = schema.type_def(type_name) else {
        return Err(vec![Issue::new(
            "note",
            format!("there is no type {type_name:?}"),
        )]);
    };
    let ctx = Context { schema, lookup };
    let mut c = Check {
        issues: &mut issues,
        // A control in the note's own content binds a field of the note's type.
        fields: Some(&ty.fields),
        template: false,
    };
    let mut report = NoteReport::default();
    let mut seen = HashSet::new();

    for (name, value) in &doc.fields {
        if !seen.insert(name.as_str()) {
            c.fail(name, "appears more than once");
            continue;
        }
        match name.as_str() {
            "title" => match value {
                Value::Str(s) if !s.trim().is_empty() => {}
                Value::Str(_) => c.fail(name, "cannot be empty"),
                v => c.fail(name, format!("must be a string, not {}", v.describe())),
            },
            "created" | "updated" => match value {
                Value::Str(s) if is_utc_datetime(s) => {}
                _ => c.fail(
                    name,
                    "must be a UTC datetime such as \"2026-10-12T09:15:00Z\"",
                ),
            },
            "conflict_of" => {
                if !value.as_str().is_some_and(is_ulid) {
                    c.fail(
                        name,
                        "must be the id of the note this is a conflict copy of",
                    );
                }
            }
            "archived" => {
                if !matches!(value, Value::Bool(_)) {
                    c.fail(
                        name,
                        format!("must be true or false, not {}", value.describe()),
                    );
                }
            }
            _ => match ty.field(name) {
                Some(f) if f.kind.is_compound() => c.fail(
                    name,
                    format!(
                        "is a {} field, written as a `{} {name}` block",
                        f.kind.name(),
                        f.kind.name()
                    ),
                ),
                Some(f) => {
                    if f.multiple && f.required && value == &Value::List(Vec::new()) {
                        c.fail(name, "is required and cannot be empty");
                    }
                    check_value(f, value, Some(&ctx), &mut c, name);
                }
                None => report.unknown.push(name.clone()),
            },
        }
    }
    if doc.field("title").is_none() {
        c.fail("title", "is required");
    }

    for b in &doc.blocks {
        let at = b.name();
        let field = match b.labels.as_slice() {
            [l] => ty.field(l),
            _ => None,
        };
        let Some(f) = field else {
            report.unknown.push(at);
            continue;
        };
        if !f.kind.is_compound() || f.kind.name() != b.kind {
            c.fail(&at, format!("{:?} is a {} field", f.name, f.kind.name()));
            continue;
        }
        if !seen.insert(f.name.as_str()) {
            c.fail(&at, "appears more than once");
            continue;
        }
        match &f.kind {
            FieldKind::Content => {
                c.items(&b.body, &[], &[], &at);
                c.content_blocks(&b.body.blocks, &at);
            }
            FieldKind::Dict { keys, values } => {
                c.only_blocks(&b.body, &[], &at);
                check_entries(keys, values, &b.body.fields, Some(&ctx), &mut c, &at);
            }
            FieldKind::Ink => c.ink(&b.body, &at),
            FieldKind::Canvas => c.canvas(&b.body, &at),
            _ => {}
        }
        if matches!(f.kind, FieldKind::Content | FieldKind::Canvas) {
            check_placeholders(schema, &b.body, &at, &mut c);
        }
    }

    for f in ty.fields.iter().filter(|f| f.required) {
        if !seen.contains(f.name.as_str()) {
            c.fail(&f.name, "is required");
        }
    }

    if issues.is_empty() {
        Ok(report)
    } else {
        Err(issues)
    }
}

/// Every `[[+type|title]]` placeholder in the markup text of `body` must name a type.
pub(crate) fn check_placeholders(schema: &Schema, body: &Doc, at: &str, c: &mut Check) {
    for (path, p) in crate::inline::placeholders_in_doc(body, at) {
        if schema.type_def(&p.type_name).is_none() {
            c.fail(&path, unknown_placeholder_type(&p));
        }
    }
}

pub(crate) fn unknown_placeholder_type(p: &crate::Placeholder) -> String {
    format!(
        "the placeholder link titled {:?} names type {:?}, which does not exist",
        p.title, p.type_name
    )
}

/// Check one non-compound field value, or a definition's `default`. Without a context, references,
/// assets and terms are checked for form only.
pub(crate) fn check_value(f: &FieldDef, v: &Value, ctx: Option<&Context>, c: &mut Check, at: &str) {
    if !f.multiple {
        return check_single(f, v, ctx, c, at);
    }
    let Value::List(items) = v else {
        return c.fail(
            at,
            format!(
                "holds several values and must be a list, not {}",
                v.describe()
            ),
        );
    };
    let mut seen = Vec::new();
    for (n, item) in items.iter().enumerate() {
        if seen.contains(&item) {
            c.fail(at, format!("lists {} twice", show(item)));
        }
        seen.push(item);
        check_single(f, item, ctx, c, &format!("{at}[{n}]"));
    }
}

fn check_single(f: &FieldDef, v: &Value, ctx: Option<&Context>, c: &mut Check, at: &str) {
    match &f.kind {
        FieldKind::String => check_scalar(Scalar::String, v, c, at),
        FieldKind::Int => check_scalar(Scalar::Int, v, c, at),
        FieldKind::Float => check_scalar(Scalar::Float, v, c, at),
        FieldKind::Bool => check_scalar(Scalar::Bool, v, c, at),
        FieldKind::Date => check_scalar(Scalar::Date, v, c, at),
        FieldKind::Datetime => check_scalar(Scalar::Datetime, v, c, at),
        FieldKind::Time => check_scalar(Scalar::Time, v, c, at),
        FieldKind::Url => check_scalar(Scalar::Url, v, c, at),
        FieldKind::DateRange { of } => date_range(*of, v, c, at),
        FieldKind::Select { options, .. } => match v {
            Value::Str(s) if options.contains(s) => {}
            Value::Str(s) => c.fail(
                at,
                format!("{s:?} is not one of the options: {}", options.join(", ")),
            ),
            v => c.fail(at, format!("must be a string, not {}", v.describe())),
        },
        FieldKind::Taxonomy { taxonomy } => match v {
            Value::Str(s) if s.is_empty() => c.fail(at, "a term path cannot be empty"),
            Value::Str(s) => {
                if let Some(ctx) = ctx {
                    match ctx.schema.taxonomy(taxonomy) {
                        None => c.fail(at, format!("there is no taxonomy {taxonomy:?}")),
                        Some(t) if !t.contains(s) => {
                            c.fail(at, format!("{s:?} is not a term of taxonomy {taxonomy:?}"))
                        }
                        Some(_) => {}
                    }
                }
            }
            v => c.fail(
                at,
                format!("must be a term path string, not {}", v.describe()),
            ),
        },
        FieldKind::Reference(r) => match v {
            Value::Str(id) if is_ulid(id) => {
                let Some(ctx) = ctx else { return };
                match ctx.lookup.note_type(id) {
                    None => c.issues.push(Issue::missing_note(at, id)),
                    Some(t) => {
                        if let Some(target) = &r.target
                            && &t != target
                        {
                            c.fail(
                                at,
                                format!("note {id} is a {t}, but this field holds a {target}"),
                            );
                        }
                    }
                }
            }
            Value::Str(id) => c.fail(at, format!("{id:?} is not a note ULID")),
            v => c.fail(
                at,
                format!("must be a note ULID string, not {}", v.describe()),
            ),
        },
        FieldKind::Asset => match v {
            Value::Str(id) if is_ulid(id) => {
                if let Some(ctx) = ctx {
                    if !ctx.lookup.asset_exists(id) {
                        c.fail(at, format!("asset {id} does not exist"));
                    } else if f.accept == Accept::Image
                        && let Some(name) = ctx.lookup.asset_filename(id)
                        && !is_image_filename(&name)
                    {
                        c.fail(
                            at,
                            format!("asset {id} is {name:?}, and this field holds only images"),
                        );
                    }
                }
            }
            Value::Str(id) => c.fail(at, format!("{id:?} is not an asset ULID")),
            v => c.fail(
                at,
                format!("must be an asset ULID string, not {}", v.describe()),
            ),
        },
        FieldKind::List { item } => match v {
            Value::List(l) => {
                for (n, x) in l.iter().enumerate() {
                    check_value(item, x, ctx, c, &format!("{at}[{n}]"));
                }
            }
            v => c.fail(at, format!("must be a list, not {}", v.describe())),
        },
        // A dict inside a dict or a list is a record of its entries.
        FieldKind::Dict { keys, values } => match v {
            Value::Record(entries) => check_entries(keys, values, entries, ctx, c, at),
            v => c.fail(
                at,
                format!("must be a record of entries, not {}", v.describe()),
            ),
        },
        FieldKind::Location { image } => {
            let (a, b, ranges) = if image.is_some() {
                ("x", "y", None)
            } else {
                ("lat", "lng", Some((90.0, 180.0)))
            };
            let Some([x, y]) = record(v, [a, b], c, at) else {
                return;
            };
            let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else {
                return c.fail(at, format!("`{a}` and `{b}` must be numbers"));
            };
            if let Some((lat, lng)) = ranges {
                if !(-lat..=lat).contains(&x) {
                    c.fail(&format!("{at}.lat"), "must be from -90 to 90");
                }
                if !(-lng..=lng).contains(&y) {
                    c.fail(&format!("{at}.lng"), "must be from -180 to 180");
                }
            } else if !x.is_finite() || !y.is_finite() {
                c.fail(at, "`x` and `y` must be finite");
            }
        }
        FieldKind::Content | FieldKind::Ink | FieldKind::Canvas => c.fail(
            at,
            format!(
                "a {} field is written as a block, not a value",
                f.kind.name()
            ),
        ),
    }
}

/// A dict's entries (types T4): a declared key by its own definition, any other by the dict's
/// `values`, and every required key there.
fn check_entries(
    keys: &[FieldDef],
    values: &DictValues,
    entries: &[(String, Value)],
    ctx: Option<&Context>,
    c: &mut Check,
    at: &str,
) {
    let mut seen = HashSet::new();
    for (k, v) in entries {
        let path = format!("{at}.{k}");
        if !seen.insert(k.as_str()) {
            c.fail(&path, "appears more than once");
            continue;
        }
        match (keys.iter().find(|d| &d.name == k), values) {
            (Some(def), _) => {
                if def.multiple && def.required && v == &Value::List(Vec::new()) {
                    c.fail(&path, "is required and cannot be empty");
                }
                check_value(def, v, ctx, c, &path);
            }
            (None, DictValues::Of(def)) => check_value(def, v, ctx, c, &path),
            (None, DictValues::Any) => {}
            (None, DictValues::None) => {
                let declared: Vec<&str> = keys.iter().map(|d| d.name.as_str()).collect();
                c.fail(
                    &path,
                    format!(
                        "is not a key of this dict, which takes {}",
                        declared.join(", ")
                    ),
                );
            }
        }
    }
    for k in keys.iter().filter(|k| k.required) {
        if !seen.contains(k.name.as_str()) {
            c.fail(&format!("{at}.{}", k.name), "is required");
        }
    }
}

pub(crate) fn check_scalar(kind: Scalar, v: &Value, c: &mut Check, at: &str) {
    let problem = match (kind, v) {
        (Scalar::String, Value::Str(_)) => None,
        (Scalar::Int, Value::Int(_)) => None,
        (Scalar::Float, Value::Int(_) | Value::Float(_)) => None,
        (Scalar::Bool, Value::Bool(_)) => None,
        (Scalar::Date, Value::Str(s)) if is_date(s) => None,
        (Scalar::Date, Value::Str(s)) => {
            Some(format!("{s:?} is not an ISO 8601 date (YYYY-MM-DD)"))
        }
        (Scalar::Datetime, Value::Str(s)) if is_datetime(s) => None,
        (Scalar::Datetime, Value::Str(s)) => Some(format!(
            "{s:?} is not an ISO 8601 datetime with seconds and an offset"
        )),
        (Scalar::Time, Value::Str(s)) if is_time(s) => None,
        (Scalar::Time, Value::Str(s)) => Some(format!(
            "{s:?} is not a 24-hour time of day (HH:MM or HH:MM:SS)"
        )),
        (Scalar::Url, Value::Str(s)) if is_url(s) => None,
        (Scalar::Url, Value::Str(s)) => Some(format!("{s:?} is not an absolute URL")),
        (Scalar::Int, Value::Float(_)) => Some("must be an integer, not a float".to_owned()),
        (Scalar::Int | Scalar::Float, v) => Some(format!("must be a number, not {}", v.describe())),
        (Scalar::Bool, v) => Some(format!("must be true or false, not {}", v.describe())),
        (Scalar::String | Scalar::Date | Scalar::Datetime | Scalar::Time | Scalar::Url, v) => {
            Some(format!("must be a string, not {}", v.describe()))
        }
    };
    if let Some(p) = problem {
        c.fail(at, p);
    }
}

fn date_range(of: RangeOf, v: &Value, c: &mut Check, at: &str) {
    let Some([start, end]) = record(v, ["start", "end"], c, at) else {
        return;
    };
    // Each end as `(seconds, is a datetime)`, read in the forms `of` allows. A date is the start
    // of its day, so the two forms order against each other.
    let parse = |v: &Value| -> Option<(f64, bool)> {
        let s = v.as_str()?;
        let date = || parse_date(s).map(|d| (d as f64 * 86_400.0, false));
        let datetime = || parse_datetime(s).map(|t| (t, true));
        match of {
            RangeOf::Date => date(),
            RangeOf::Datetime => datetime(),
            RangeOf::DateOrDatetime => date().or_else(datetime),
        }
    };
    let what = match of {
        RangeOf::Date => "date",
        RangeOf::Datetime => "datetime",
        RangeOf::DateOrDatetime => "date or datetime",
    };
    match (parse(start), parse(end)) {
        (Some((_, a)), Some((_, b))) if a != b => c.fail(
            &format!("{at}.end"),
            if a {
                "must be a datetime, as start is: both ends are dates or both are datetimes"
            } else {
                "must be a date, as start is: both ends are dates or both are datetimes"
            },
        ),
        (Some((s, _)), Some((e, _))) if e < s => c.fail(&format!("{at}.end"), "is before start"),
        (Some(_), Some(_)) => {}
        (s, e) => {
            if s.is_none() {
                c.fail(
                    &format!("{at}.start"),
                    format!("must be an ISO 8601 {what}"),
                );
            }
            if e.is_none() {
                c.fail(&format!("{at}.end"), format!("must be an ISO 8601 {what}"));
            }
        }
    }
}

/// A record holding exactly `keys`, in any order. WCL does not check record members, so a missing
/// or extra key is caught here.
fn record<'v, const N: usize>(
    v: &'v Value,
    keys: [&str; N],
    c: &mut Check,
    at: &str,
) -> Option<[&'v Value; N]> {
    let Value::Record(members) = v else {
        c.fail(
            at,
            format!(
                "must be a record {{ {} }}, not {}",
                keys.join(", "),
                v.describe()
            ),
        );
        return None;
    };
    let mut ok = true;
    for (k, _) in members {
        if !keys.contains(&k.as_str()) {
            c.fail(at, format!("has an unexpected key `{k}`"));
            ok = false;
        }
    }
    let mut out = Vec::with_capacity(N);
    for k in keys {
        let found: Vec<_> = members.iter().filter(|(m, _)| m == k).collect();
        match found.as_slice() {
            [(_, v)] => out.push(v),
            [] => {
                c.fail(at, format!("is missing key `{k}`"));
                ok = false;
            }
            _ => {
                c.fail(at, format!("repeats key `{k}`"));
                ok = false;
            }
        }
    }
    if ok { out.try_into().ok() } else { None }
}

fn show(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("{s:?}"),
        other => other.describe().to_owned(),
    }
}
