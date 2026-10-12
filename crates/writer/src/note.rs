//! A submitted note: WCL text or JSON in (D32), one canonical note file out (D21).
//!
//! Both forms become a [`Doc`]. The writer puts the doc's items in the order of
//! `docs/vault-format.md` "Note files", prints it, and passes the text through
//! [`store::canonical`], which checks it is data-only again and fixes its layout. Labels that are
//! identifiers are printed bare and every other label is quoted, so a note reads the same whichever
//! form it was submitted in.

use schema::{Doc, DocBlock, TypeDef, Value};
use serde_json::{Map, Number, Value as Json};

use crate::{Error, Result};

/// A note as a client submitted it (step 1 of the write path).
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// WCL text.
    Wcl(String),
    /// The JSON form: `{ "fields": { name: value }, "blocks": [ block ] }`, where a block is
    /// `{ "kind": k, "labels": [..], "fields": {..}, "blocks": [..] }`. Records are objects.
    Json(Json),
}

impl Input {
    /// Parse and check the note: WCL must be data-only (step 2). `file` names it in diagnostics.
    pub fn parse(&self, file: &str) -> Result<Doc> {
        match self {
            Input::Wcl(text) => {
                let ast = store::parse_data_only(text, file).map_err(Error::from_store)?;
                Ok(Doc::from_source(&ast))
            }
            Input::Json(json) => doc_from_json(json, "note"),
        }
    }
}

/// The JSON form of a doc, the inverse of [`Input::Json`].
pub fn doc_to_json(doc: &Doc) -> Json {
    let mut out = Map::new();
    out.insert("fields".into(), fields_to_json(&doc.fields));
    out.insert(
        "blocks".into(),
        Json::Array(doc.blocks.iter().map(block_to_json).collect()),
    );
    Json::Object(out)
}

fn fields_to_json(fields: &[(String, Value)]) -> Json {
    Json::Object(
        fields
            .iter()
            .map(|(n, v)| (n.clone(), value_to_json(v)))
            .collect(),
    )
}

/// The JSON form of one block, as in [`Input::Json`].
pub fn block_to_json(b: &DocBlock) -> Json {
    let mut out = Map::new();
    out.insert("kind".into(), Json::String(b.kind.clone()));
    if !b.labels.is_empty() {
        out.insert(
            "labels".into(),
            Json::Array(b.labels.iter().cloned().map(Json::String).collect()),
        );
    }
    if !b.body.fields.is_empty() {
        out.insert("fields".into(), fields_to_json(&b.body.fields));
    }
    if !b.body.blocks.is_empty() {
        out.insert(
            "blocks".into(),
            Json::Array(b.body.blocks.iter().map(block_to_json).collect()),
        );
    }
    Json::Object(out)
}

/// The JSON form of one value, as [`doc_to_json`] writes it.
pub fn value_to_json(v: &Value) -> Json {
    match v {
        Value::Str(s) => Json::String(s.clone()),
        Value::Int(i) => Json::Number((*i).into()),
        Value::Float(f) => Number::from_f64(*f).map_or(Json::Null, Json::Number),
        Value::Bool(b) => Json::Bool(*b),
        Value::List(l) => Json::Array(l.iter().map(value_to_json).collect()),
        Value::Record(members) => Json::Object(
            members
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect(),
        ),
    }
}

fn doc_from_json(json: &Json, at: &str) -> Result<Doc> {
    let bad = |message: String| Err(Error::BadJson(format!("{at}: {message}")));
    let Json::Object(obj) = json else {
        return bad("must be an object with `fields` and `blocks`".into());
    };
    let mut doc = Doc::default();
    for (key, value) in obj {
        match key.as_str() {
            "fields" => {
                let Json::Object(fields) = value else {
                    return bad("`fields` must be an object".into());
                };
                for (name, v) in fields {
                    let path = format!("{at}.{name}");
                    if !is_identifier(name) {
                        return bad(format!("{name:?} is not a field name"));
                    }
                    doc.fields.push((name.clone(), value_from_json(v, &path)?));
                }
            }
            "blocks" => {
                let Json::Array(blocks) = value else {
                    return bad("`blocks` must be a list".into());
                };
                for (n, b) in blocks.iter().enumerate() {
                    doc.blocks
                        .push(block_from_json(b, &format!("{at}.blocks[{n}]"))?);
                }
            }
            other => return bad(format!("unexpected key {other:?}")),
        }
    }
    Ok(doc)
}

fn block_from_json(json: &Json, at: &str) -> Result<DocBlock> {
    let bad = |message: &str| Err(Error::BadJson(format!("{at}: {message}")));
    let Json::Object(obj) = json else {
        return bad("a block must be an object");
    };
    let Some(Json::String(kind)) = obj.get("kind") else {
        return bad("a block needs a string `kind`");
    };
    if !is_identifier(kind) {
        return bad("`kind` is not an identifier");
    }
    let labels = match obj.get("labels") {
        None => Vec::new(),
        Some(Json::Array(labels)) => labels
            .iter()
            .map(|l| match l {
                Json::String(s) if !s.contains('\n') => Ok(s.clone()),
                _ => Err(Error::BadJson(format!(
                    "{at}: every label must be a one-line string"
                ))),
            })
            .collect::<Result<_>>()?,
        Some(_) => return bad("`labels` must be a list of strings"),
    };
    let mut body = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "kind" | "labels" => {}
            "fields" | "blocks" => {
                body.insert(key.clone(), value.clone());
            }
            other => return bad(&format!("unexpected key {other:?}")),
        }
    }
    Ok(DocBlock {
        kind: kind.clone(),
        labels,
        body: doc_from_json(&Json::Object(body), at)?,
    })
}

fn value_from_json(json: &Json, at: &str) -> Result<Value> {
    let bad = |message: &str| Err(Error::BadJson(format!("{at}: {message}")));
    Ok(match json {
        Json::String(s) => Value::Str(s.clone()),
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) if !n.is_f64() => Value::Int(i),
            (_, Some(f)) if n.is_f64() => Value::Float(f),
            _ => return bad("the number does not fit in a signed 64-bit integer"),
        },
        Json::Array(items) => Value::List(
            items
                .iter()
                .enumerate()
                .map(|(n, v)| value_from_json(v, &format!("{at}[{n}]")))
                .collect::<Result<_>>()?,
        ),
        Json::Object(members) => Value::Record(
            members
                .iter()
                .map(|(k, v)| {
                    if is_identifier(k) {
                        Ok((k.clone(), value_from_json(v, &format!("{at}.{k}"))?))
                    } else {
                        Err(Error::BadJson(format!(
                            "{at}: record key {k:?} is not an identifier"
                        )))
                    }
                })
                .collect::<Result<_>>()?,
        ),
        Json::Null => return bad("null is not a value; leave the field out instead"),
    })
}

/// `[A-Za-z_][A-Za-z0-9_]*`: a field name, block kind or record key.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Put a note's items in canonical order: `title`, `created`, `updated`, then the type's fields in
/// type-file order, then fields the type does not describe; compound blocks in type-file order,
/// then the rest. Any `created` or `updated` the client sent is replaced by the server's.
pub(crate) fn arrange(ty: &TypeDef, doc: Doc, created: &str, updated: &str) -> Doc {
    let mut fields: Vec<Option<(String, Value)>> = doc.fields.into_iter().map(Some).collect();
    let mut take = |name: &str, out: &mut Vec<(String, Value)>| {
        for slot in fields.iter_mut() {
            if slot.as_ref().is_some_and(|(n, _)| n == name) {
                out.extend(slot.take());
            }
        }
    };
    let mut ordered = Vec::new();
    take("title", &mut ordered);
    let mut dropped = Vec::new();
    take("created", &mut dropped);
    take("updated", &mut dropped);
    ordered.push(("created".to_owned(), Value::Str(created.to_owned())));
    ordered.push(("updated".to_owned(), Value::Str(updated.to_owned())));
    for f in ty.fields.iter().filter(|f| !f.kind.is_compound()) {
        take(&f.name, &mut ordered);
    }
    ordered.extend(fields.into_iter().flatten());

    let mut blocks: Vec<Option<DocBlock>> = doc.blocks.into_iter().map(Some).collect();
    let mut ordered_blocks = Vec::new();
    for f in ty.fields.iter().filter(|f| f.kind.is_compound()) {
        for slot in blocks.iter_mut() {
            if slot.as_ref().is_some_and(|b| b.label() == Some(&f.name)) {
                ordered_blocks.extend(slot.take());
            }
        }
    }
    ordered_blocks.extend(blocks.into_iter().flatten());

    Doc {
        fields: ordered,
        blocks: ordered_blocks,
    }
}

/// Print a doc as WCL text, ready for [`store::canonical`].
pub(crate) fn print(doc: &Doc) -> String {
    let mut out = String::new();
    print_body(&mut out, doc);
    out
}

fn print_body(out: &mut String, doc: &Doc) {
    for (name, value) in &doc.fields {
        out.push_str(name);
        out.push_str(" = ");
        print_value(out, value);
        out.push('\n');
    }
    for b in &doc.blocks {
        out.push_str(&b.kind);
        for label in &b.labels {
            out.push(' ');
            if is_identifier(label) {
                out.push_str(label);
            } else {
                quote(out, label);
            }
        }
        out.push_str(" {\n");
        print_body(out, &b.body);
        out.push_str("}\n");
    }
}

fn print_value(out: &mut String, v: &Value) {
    match v {
        Value::Str(s) => quote(out, s),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&format!("{f:?}")),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::List(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                print_value(out, item);
            }
            out.push(']');
        }
        Value::Record(members) => {
            out.push('{');
            for (n, (k, v)) in members.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                out.push(' ');
                out.push_str(k);
                out.push_str(": ");
                print_value(out, v);
            }
            out.push_str(" }");
        }
    }
}

fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips_through_a_doc() {
        let json = serde_json::json!({
            "fields": { "title": "T", "n": 3, "x": 1.5, "ok": false, "tags": ["a", "b"],
                        "at": { "lat": 1.0, "lng": 2.0 } },
            "blocks": [{ "kind": "content", "labels": ["body"],
                         "blocks": [{ "kind": "paragraph", "fields": { "text": "hi" } }] }]
        });
        let doc = Input::Json(json.clone()).parse("n.wcl").unwrap();
        assert_eq!(doc_to_json(&doc), json);
    }

    #[test]
    fn json_rejects_names_that_would_escape_the_text() {
        let json = serde_json::json!({ "fields": { "a = 1\nb": 2 } });
        assert!(matches!(
            Input::Json(json).parse("n.wcl"),
            Err(Error::BadJson(_))
        ));
        let json = serde_json::json!({ "fields": { "a": null } });
        assert!(matches!(
            Input::Json(json).parse("n.wcl"),
            Err(Error::BadJson(_))
        ));
    }

    #[test]
    fn printed_text_reads_back_as_the_same_doc() {
        let text = "title = \"Quote \\\" and \\\\ and\\nnewline\"\nf = 2.0\nr = { a: 1, b: [true] }\nview \"not ident\" open {\n  kind = \"table\"\n}\n";
        let doc = Input::Wcl(text.into()).parse("n.wcl").unwrap();
        let again = Input::Wcl(print(&doc)).parse("n.wcl").unwrap();
        assert_eq!(doc, again);
    }
}
