//! A note turned into index rows: its field values, its links and its full-text document.

use schema::{Doc, DocBlock, FieldKind, NoteReport, TypeDef, Value};

/// One note, ready to insert.
pub(crate) struct Note {
    pub id: String,
    pub type_name: String,
    pub title: String,
    pub created: Option<String>,
    pub updated: Option<String>,
    /// `created` and `updated` as seconds since the Unix epoch, when they read as datetimes.
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub flagged: bool,
    /// Every top-level field, as a JSON object in file order.
    pub data: String,
    pub values: Vec<FieldValue>,
    pub links: Vec<Link>,
    pub search: SearchDoc,
}

/// One scalar inside a field value. `pos` is the position in a list, and `part` names the member
/// of a record or the key of a dict, empty for a plain scalar.
pub(crate) struct FieldValue {
    pub field: String,
    pub pos: i64,
    pub part: String,
    pub kind: &'static str,
    pub str: Option<String>,
    pub num: Option<Num>,
}

#[derive(Clone, Copy)]
pub(crate) enum Num {
    Int(i64),
    Real(f64),
}

/// A link from this note. `via` is `field` for a reference field and `mention` for a mention in
/// inline text, where `field` is the content or canvas field holding it.
pub(crate) struct Link {
    pub field: String,
    pub pos: i64,
    pub dst: String,
    pub via: &'static str,
}

pub(crate) struct SearchDoc {
    pub id: String,
    pub type_name: String,
    pub title: String,
    pub body: String,
}

pub(crate) fn project(ty: &TypeDef, id: &str, doc: &Doc, report: &NoteReport) -> Note {
    let str_field = |name: &str| doc.field(name).and_then(Value::as_str).map(str::to_owned);
    let title = str_field("title").unwrap_or_default();
    let mut values = Vec::new();
    let mut links = Vec::new();
    let mut body = Vec::new();

    for (name, value) in &doc.fields {
        let kind = match name.as_str() {
            "title" => Kind::Text,
            "created" | "updated" => Kind::Datetime,
            _ => match ty.field(name) {
                Some(f) => Kind::of(&f.kind),
                None => continue,
            },
        };
        let items: Vec<&Value> = match value {
            Value::List(l) => l.iter().collect(),
            v => vec![v],
        };
        for (pos, item) in items.into_iter().enumerate() {
            let pos = pos as i64;
            if name != "title"
                && kind.searchable()
                && let Some(s) = item.as_str()
            {
                body.push(s.to_owned());
            }
            if kind == Kind::Reference
                && let Some(dst) = item.as_str()
            {
                links.push(Link {
                    field: name.clone(),
                    pos,
                    dst: dst.to_owned(),
                    via: "field",
                });
            }
            scalars(&mut values, name, pos, "", kind, item);
        }
    }

    for b in &doc.blocks {
        let Some(f) = b.label().and_then(|l| ty.field(l)) else {
            continue;
        };
        if f.kind.name() != b.kind {
            continue;
        }
        match &f.kind {
            FieldKind::Dict { values: v } => {
                let kind = Kind::of_scalar(*v);
                for (key, item) in &b.body.fields {
                    if kind.searchable()
                        && let Some(s) = item.as_str()
                    {
                        body.push(s.to_owned());
                    }
                    scalars(&mut values, &f.name, 0, key, kind, item);
                }
            }
            FieldKind::Content | FieldKind::Canvas => {
                let mut mentioned = Vec::new();
                block_text(&b.body, &mut body, &mut mentioned);
                for (pos, dst) in mentioned.into_iter().enumerate() {
                    links.push(Link {
                        field: f.name.clone(),
                        pos: pos as i64,
                        dst,
                        via: "mention",
                    });
                }
            }
            _ => {}
        }
    }

    let data = serde_json::Value::Object(
        doc.fields
            .iter()
            .map(|(k, v)| (k.clone(), json(v)))
            .collect(),
    );
    Note {
        id: id.to_owned(),
        type_name: ty.name.clone(),
        search: SearchDoc {
            id: id.to_owned(),
            type_name: ty.name.clone(),
            title: title.clone(),
            body: body.join("\n"),
        },
        title,
        created: str_field("created"),
        updated: str_field("updated"),
        created_at: str_field("created").as_deref().and_then(time::datetime),
        updated_at: str_field("updated").as_deref().and_then(time::datetime),
        flagged: report.is_flagged(),
        data: data.to_string(),
        values,
        links,
    }
}

/// How a field's values are stored. `num` holds numbers, booleans as 0 or 1, and dates and
/// datetimes as seconds since the Unix epoch, so they compare in time order whatever the offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Int,
    Float,
    Bool,
    Date,
    Datetime,
    DateRange,
    Url,
    Select,
    Taxonomy,
    Reference,
    Location,
    Asset,
}

impl Kind {
    fn of(kind: &FieldKind) -> Kind {
        match kind {
            FieldKind::String => Kind::Text,
            FieldKind::Int => Kind::Int,
            FieldKind::Float => Kind::Float,
            FieldKind::Bool => Kind::Bool,
            FieldKind::Date => Kind::Date,
            FieldKind::Datetime => Kind::Datetime,
            FieldKind::DateRange { .. } => Kind::DateRange,
            FieldKind::Url => Kind::Url,
            FieldKind::Select { .. } => Kind::Select,
            FieldKind::Taxonomy { .. } => Kind::Taxonomy,
            FieldKind::Reference(_) => Kind::Reference,
            FieldKind::List { items } => Kind::of_scalar(*items),
            FieldKind::Location { .. } => Kind::Location,
            FieldKind::Asset => Kind::Asset,
            FieldKind::Dict { values } => Kind::of_scalar(*values),
            FieldKind::Content | FieldKind::Ink | FieldKind::Canvas => Kind::Text,
        }
    }

    fn of_scalar(s: schema::Scalar) -> Kind {
        use schema::Scalar;
        match s {
            Scalar::String => Kind::Text,
            Scalar::Int => Kind::Int,
            Scalar::Float => Kind::Float,
            Scalar::Bool => Kind::Bool,
            Scalar::Date => Kind::Date,
            Scalar::Datetime => Kind::Datetime,
            Scalar::Url => Kind::Url,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Text => "string",
            Kind::Int => "int",
            Kind::Float => "float",
            Kind::Bool => "bool",
            Kind::Date => "date",
            Kind::Datetime => "datetime",
            Kind::DateRange => "date_range",
            Kind::Url => "url",
            Kind::Select => "select",
            Kind::Taxonomy => "taxonomy",
            Kind::Reference => "reference",
            Kind::Location => "location",
            Kind::Asset => "asset",
        }
    }

    /// Whether the value's text goes into the full-text index.
    fn searchable(self) -> bool {
        matches!(self, Kind::Text | Kind::Url | Kind::Select | Kind::Taxonomy)
    }
}

fn scalars(out: &mut Vec<FieldValue>, field: &str, pos: i64, part: &str, kind: Kind, v: &Value) {
    if let Value::Record(members) = v {
        for (key, member) in members {
            scalars(out, field, pos, key, kind, member);
        }
        return;
    }
    let (str, num) = match v {
        Value::Str(s) => {
            let num = match kind {
                Kind::Date | Kind::DateRange if s.len() == 10 => {
                    time::date(s).map(|d| Num::Int(d * 86_400))
                }
                Kind::Datetime | Kind::DateRange => time::datetime(s).map(Num::Real),
                _ => None,
            };
            (Some(s.clone()), num)
        }
        Value::Int(i) if kind == Kind::Float => (None, Some(Num::Real(*i as f64))),
        Value::Int(i) => (None, Some(Num::Int(*i))),
        Value::Float(f) => (None, Some(Num::Real(*f))),
        Value::Bool(b) => (None, Some(Num::Int(i64::from(*b)))),
        Value::List(_) | Value::Record(_) => return,
    };
    out.push(FieldValue {
        field: field.to_owned(),
        pos,
        part: part.to_owned(),
        kind: kind.name(),
        str,
        num,
    });
}

/// Items of content and canvas blocks whose text is searched. `text` items in the blocks named
/// by [`MARKUP_BLOCKS`] are inline markup and may hold mentions.
const TEXT_ITEMS: [&str; 9] = [
    "text", "source", "line", "stdout", "stderr", "alt", "caption", "title", "label",
];
const MARKUP_BLOCKS: [&str; 4] = ["heading", "paragraph", "item", "quote"];

fn block_text(body: &Doc, text: &mut Vec<String>, mentioned: &mut Vec<String>) {
    for b in &body.blocks {
        visit(b, text, mentioned);
    }
}

fn visit(b: &DocBlock, text: &mut Vec<String>, mentioned: &mut Vec<String>) {
    let markup = MARKUP_BLOCKS.contains(&b.kind.as_str());
    for (name, v) in &b.body.fields {
        let Some(s) = v.as_str() else { continue };
        if !TEXT_ITEMS.contains(&name.as_str()) {
            continue;
        }
        text.push(s.to_owned());
        if markup && name == "text" {
            mentioned.extend(mentions(s));
        }
    }
    block_text(&b.body, text, mentioned);
}

/// The note ids mentioned in inline markup (PRD Appendix A), in order: `[[ULID]]` and
/// `[[ULID|label]]`. Escaped brackets and inline code are skipped.
pub fn mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    let mut in_code = false;
    while let Some(c) = rest.chars().next() {
        match c {
            '\\' => {
                let mut it = rest.chars();
                it.next();
                it.next();
                rest = it.as_str();
                continue;
            }
            '`' => in_code = !in_code,
            '[' if !in_code && rest.starts_with("[[") => {
                let inner = &rest[2..];
                if let Some(end) = inner.find("]]") {
                    let target = &inner[..end];
                    let id = target.split('|').next().unwrap_or("");
                    if schema::is_ulid(id) {
                        out.push(id.to_owned());
                        rest = &inner[end + 2..];
                        continue;
                    }
                }
            }
            _ => {}
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn json(v: &Value) -> serde_json::Value {
    match v {
        Value::Str(s) => s.clone().into(),
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Bool(b) => (*b).into(),
        Value::List(l) => l.iter().map(json).collect(),
        Value::Record(r) => {
            serde_json::Value::Object(r.iter().map(|(k, v)| (k.clone(), json(v))).collect())
        }
    }
}

/// Dates and datetimes as Unix time. Values reaching the index have passed validation, so a
/// value these cannot read is simply stored without a number.
mod time {
    /// Days since 1970-01-01 of `YYYY-MM-DD`.
    pub fn date(s: &str) -> Option<i64> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        Some(days_from_civil(
            digits(&s[0..4])?,
            digits(&s[5..7])?,
            digits(&s[8..10])?,
        ))
    }

    /// Seconds since the Unix epoch of `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
    pub fn datetime(s: &str) -> Option<f64> {
        if s.len() < 20 || s.as_bytes()[10] != b'T' {
            return None;
        }
        let days = date(&s[..10])?;
        let t = &s[11..];
        let (h, mi, sec) = (
            digits(t.get(0..2)?)?,
            digits(t.get(3..5)?)?,
            digits(t.get(6..8)?)?,
        );
        let mut rest = t.get(8..)?;
        let mut frac = 0.0;
        if let Some(after) = rest.strip_prefix('.') {
            let n = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            frac = format!("0.{}", &after[..n]).parse().ok()?;
            rest = &after[n..];
        }
        let offset = match rest.as_bytes().first()? {
            b'Z' => 0,
            sign @ (b'+' | b'-') => {
                let o = &rest[1..];
                let secs = digits(o.get(0..2)?)? * 3600 + digits(o.get(3..5)?)? * 60;
                if *sign == b'+' { secs } else { -secs }
            }
            _ => return None,
        };
        Some((days * 86_400 + h * 3600 + mi * 60 + sec - offset) as f64 + frac)
    }

    fn digits(s: &str) -> Option<i64> {
        if s.bytes().all(|c| c.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    }

    /// Howard Hinnant's days-from-civil.
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    const B: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

    #[test]
    fn finds_mentions_with_and_without_labels() {
        let text = format!("see [[{A}]] and [[{B}|the other]].");
        assert_eq!(mentions(&text), vec![A, B]);
    }

    #[test]
    fn skips_escaped_and_code_mentions_and_non_ulids() {
        let text = format!("\\[[{A}]] `[[{A}]]` [[not a ulid]] [[{B}]]");
        assert_eq!(mentions(&text), vec![B]);
    }

    #[test]
    fn datetimes_with_offsets_compare_in_time_order() {
        let utc = time::datetime("2026-10-20T00:00:00Z").unwrap();
        let east = time::datetime("2026-10-20T09:30:00+11:00").unwrap();
        assert!(east < utc);
        assert_eq!(utc - east, 1.5 * 3600.0);
        assert_eq!(time::date("1970-01-02"), Some(1));
        assert_eq!(time::datetime("1970-01-01T00:00:01.5Z"), Some(1.5));
    }
}
