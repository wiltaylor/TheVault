//! Type definitions. Creating a type, or replacing one that has no notes, writes its file at once;
//! changing a type that has notes needs a planned schema change (D6).
//!
//! `GET /api/types` lists every type with its fields and templates as JSON, which is what the web
//! client lays a note out from (D14).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use schema::{Doc, DocBlock, FieldDef, FieldKind, Relation, Scalar, TabItem, Template, TypeDef};
use serde_json::{Value, json};
use writer::{Writer, doc_to_json};

use crate::notes::{is_json, wants_wcl};
use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

fn type_response(headers: &HeaderMap, name: &str, wcl: String) -> Response {
    if wants_wcl(headers) {
        let mut res = wcl.into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        return res;
    }
    Json(json!({ "name": name, "wcl": wcl })).into_response()
}

/// `GET /api/types`: every type, by name, with its fields and templates.
pub async fn list(State(w): Shared) -> Json<Value> {
    let schema = w.schema();
    let mut types: Vec<&TypeDef> = schema.types().collect();
    types.sort_by(|a, b| a.name.cmp(&b.name));
    Json(json!({ "types": types.into_iter().map(type_json).collect::<Vec<_>>() }))
}

fn type_json(t: &TypeDef) -> Value {
    json!({
        "name": t.name,
        "label": t.label,
        "plural": t.plural,
        "description": t.description,
        "icon": t.icon,
        "colour": t.colour,
        "fields": t.fields.iter().map(field_json).collect::<Vec<_>>(),
        "templates": t.templates.iter().map(template_json).collect::<Vec<_>>(),
    })
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

fn field_json(f: &FieldDef) -> Value {
    let mut out = json!({
        "name": f.name,
        "label": f.label,
        "description": f.description,
        "kind": f.kind.name(),
        "required": f.required,
        "computed": f.computed,
        "multiple": f.multiple,
        "compound": f.kind.is_compound(),
    });
    if let Some(default) = &f.default {
        let doc = Doc {
            fields: vec![("default".to_owned(), default.clone())],
            blocks: Vec::new(),
        };
        out["default"] = doc_to_json(&doc)["fields"]["default"].take();
    }
    let extra = match &f.kind {
        FieldKind::DateRange { datetime } => {
            json!({ "of": if *datetime { "datetime" } else { "date" } })
        }
        FieldKind::Select { options } => json!({ "options": options }),
        FieldKind::Taxonomy { taxonomy } => json!({ "taxonomy": taxonomy }),
        FieldKind::Reference(r) => json!({
            "target": r.target.as_deref().unwrap_or("*"),
            "relation": match r.relation {
                Relation::OneWay => "one_way",
                Relation::TwoWay => "two_way",
                Relation::Peer => "peer",
            },
            "inverse": r.inverse,
        }),
        FieldKind::List { items } => json!({ "items": scalar_name(*items) }),
        FieldKind::Dict { values } => json!({ "values": scalar_name(*values) }),
        FieldKind::Location { image } => json!({ "image": image }),
        _ => json!({}),
    };
    if let (Value::Object(out), Value::Object(extra)) = (&mut out, extra) {
        out.extend(extra);
    }
    out
}

fn block_json(b: &DocBlock) -> Value {
    let doc = Doc {
        fields: Vec::new(),
        blocks: vec![b.clone()],
    };
    doc_to_json(&doc)["blocks"][0].take()
}

fn template_json(t: &Template) -> Value {
    let tabs: Vec<Value> = t
        .tabs
        .iter()
        .map(|tab| {
            let items: Vec<Value> = tab
                .items
                .iter()
                .map(|item| match item {
                    TabItem::Slot { field, label } => {
                        json!({ "slot": field, "label": label })
                    }
                    TabItem::Block(b) => json!({ "block": block_json(b) }),
                })
                .collect();
            json!({ "title": tab.title, "items": items })
        })
        .collect();
    json!({ "name": t.name, "label": t.label, "tabs": tabs })
}

/// `GET /api/types/{name}`: the type's definition as stored.
pub async fn read(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let n = name.clone();
    let wcl = blocking(w, move |w| w.read_type(&n)).await?;
    Ok(type_response(&headers, &name, wcl))
}

/// `PUT /api/types/{name}`: create or replace a type from its WCL, sent as text or as
/// `{ "wcl": "..." }`.
pub async fn put(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let source = if is_json(&headers) {
        let value: Value = serde_json::from_slice(&body)
            .map_err(|e| ApiError::bad_request(format!("the body is not JSON: {e}")))?;
        match value.as_object().map(|o| (o.len(), o.get("wcl"))) {
            Some((1, Some(Value::String(s)))) => s.clone(),
            _ => return Err(ApiError::bad_request("send `{ \"wcl\": \"...\" }`")),
        }
    } else {
        String::from_utf8(body.to_vec())
            .map_err(|_| ApiError::bad_request("the WCL body is not UTF-8"))?
    };
    let n = name.clone();
    let wcl = blocking(w, move |w| w.put_type(&n, &source)).await?;
    Ok(type_response(&headers, &name, wcl))
}
