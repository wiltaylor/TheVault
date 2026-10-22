//! Type definitions, planned schema changes and flagged notes (D6). Every route but the reads
//! needs the `admin` scope (D11).
//!
//! - `PUT /api/types/{name}` creates a type, or replaces one that has no notes, at once. The JSON
//!   answer carries `warnings`: what this version ignores in the file, such as an attribute of
//!   the type or a field that it does not know. `GET /api/types` and `GET /api/schema` list each
//!   type's `warnings` too.
//! - `POST /api/types/{name}/change` takes `{ "wcl", "renames"?, "migration"?, "plan"? }` and
//!   changes a type that has notes. Without `plan` it returns the plan and changes nothing. With
//!   the `plan` token from that response it applies the change as one commit, or answers 409
//!   `plan_stale` with the plan as it is now. `migration` is `{ "script", "function"? }`: function
//!   `function`, by default `migrate`, of `scripts/<script>.ws` runs over every note of the type
//!   (D6). A migration that fails on a note answers 422 `migration_failed` naming the note.
//! - `DELETE /api/types/{name}` deletes a type that has no notes.
//! - `GET /api/flagged` lists notes holding data their type no longer describes.
//! - `GET /api/types` lists every type with its fields and templates as JSON, which is what the
//!   web client lays a note out from (D14).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use schema::{Doc, DocBlock, FieldDef, FieldKind, Relation, Scalar, TabItem, Template, TypeDef};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{Migration, NoteEdit, TypeChange, TypePlan, Writer, doc_to_json};

use crate::error::issue_json;
use crate::notes::{is_json, wants_wcl};
use crate::{ApiError, Caller, blocking};

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
    let types: Vec<Value> = types
        .into_iter()
        .map(|t| type_json(t, w.is_built_in(&t.name)))
        .collect();
    Json(json!({ "types": types }))
}

fn type_json(t: &TypeDef, builtin: bool) -> Value {
    json!({
        "name": t.name,
        "label": t.label,
        "plural": t.plural,
        "description": t.description,
        "icon": t.icon,
        "colour": t.colour,
        "builtin": builtin,
        "fields": t.fields.iter().map(field_json).collect::<Vec<_>>(),
        "templates": t.templates.iter().map(template_json).collect::<Vec<_>>(),
        "warnings": t.warnings.iter().map(issue_json).collect::<Vec<_>>(),
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
        "hidden": f.hidden,
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
        FieldKind::DateRange { of } => json!({ "of": of.name() }),
        FieldKind::Select {
            options,
            option_labels,
        } if !option_labels.is_empty() => {
            json!({ "options": options, "option_labels": option_labels })
        }
        FieldKind::Select { options, .. } => json!({ "options": options }),
        // A list of terms reads as a multiple taxonomy field; `items` says how it was declared.
        FieldKind::Taxonomy { taxonomy } if f.term_list => {
            json!({ "taxonomy": taxonomy, "items": "taxonomy" })
        }
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
        FieldKind::List { items } => json!({ "items": scalar_name(*items), "labels": f.labels }),
        FieldKind::Asset => json!({ "accept": f.accept.name() }),
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
                    TabItem::Slot { field, label, .. } => {
                        json!({ "slot": field, "label": label })
                    }
                    TabItem::Block(b) => json!({ "block": block_json(b) }),
                })
                .collect();
            let mut out = json!({ "title": tab.title, "items": items });
            if let Some(w) = &tab.when {
                out["when"] = json!(w);
            }
            out
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
    let (wcl, warnings) = blocking(w, move |w| {
        let wcl = w.put_type(&n, &source)?;
        let warnings = w
            .schema()
            .type_def(&n)
            .map(|t| t.warnings.clone())
            .unwrap_or_default();
        Ok::<_, writer::Error>((wcl, warnings))
    })
    .await?;
    if wants_wcl(&headers) {
        return Ok(type_response(&headers, &name, wcl));
    }
    // What this version ignores in the type, such as an attribute it does not know, so that a
    // typo shows.
    let warnings: Vec<Value> = warnings.iter().map(issue_json).collect();
    Ok(Json(json!({ "name": name, "wcl": wcl, "warnings": warnings })).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeBody {
    wcl: String,
    #[serde(default)]
    renames: BTreeMap<String, String>,
    migration: Option<MigrationBody>,
    plan: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationBody {
    script: String,
    function: Option<String>,
}

/// `POST /api/types/{name}/change`: plan a change, or apply a planned one.
pub async fn change(
    State(w): Shared,
    Caller(author): Caller,
    Path(name): Path<String>,
    Json(body): Json<ChangeBody>,
) -> Result<Json<Value>, ApiError> {
    let change = TypeChange {
        wcl: body.wcl,
        renames: body.renames,
        migration: body.migration.map(|m| Migration {
            script: m.script,
            function: m.function.unwrap_or_else(|| "migrate".to_owned()),
        }),
    };
    let Some(token) = body.plan else {
        let plan = blocking(w, move |w| w.plan_type_change(&name, &change))
            .await
            .map_err(missing_type)?;
        return Ok(Json(plan_json(&plan, false)));
    };
    let (plan, saved) = blocking(w, move |w| {
        w.apply_type_change(&name, &change, &token, &author)
    })
    .await
    .map_err(missing_type)?;
    let mut out = plan_json(&plan, true);
    out["notes"] = saved
        .iter()
        .map(|s| json!({ "id": s.id, "version": s.version }))
        .collect();
    Ok(Json(out))
}

/// `DELETE /api/types/{name}`.
pub async fn delete(State(w): Shared, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let name2 = name.clone();
    blocking(w, move |w| w.delete_type(&name2))
        .await
        .map_err(missing_type)?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

/// `GET /api/flagged`.
pub async fn flagged(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let notes = blocking(w, |w| w.flagged()).await?;
    let notes: Vec<Value> = notes
        .into_iter()
        .map(|n| {
            json!({
                "id": n.id,
                "type": n.type_name,
                "title": n.title,
                "version": n.version,
                "unknown": n.unknown,
            })
        })
        .collect();
    Ok(Json(json!({ "notes": notes })))
}

/// A type named in the path that does not exist is a 404, where a note naming one is a 400.
fn missing_type(mut e: ApiError) -> ApiError {
    if e.body["error"] == "unknown_type" {
        e.status = StatusCode::NOT_FOUND;
    }
    e
}

/// A plan as JSON. Its `plan` member is the token that applies it.
pub(crate) fn plan_json(plan: &TypePlan, applied: bool) -> Value {
    let changes: Vec<Value> = plan
        .changes
        .iter()
        .map(|c| {
            let edits: Vec<Value> = c
                .edits
                .iter()
                .map(|e| match e {
                    NoteEdit::Rename { from, to } => {
                        json!({ "op": "rename", "from": from, "to": to })
                    }
                    NoteEdit::Default { field, value } => json!({
                        "op": "default",
                        "field": field,
                        "value": writer::value_to_json(value),
                    }),
                    NoteEdit::Migrated { script, function } => json!({
                        "op": "migrate",
                        "script": script,
                        "function": function,
                    }),
                })
                .collect();
            json!({ "id": c.id, "title": c.title, "version": c.version, "edits": edits })
        })
        .collect();
    json!({
        "type": plan.type_name,
        "applied": applied,
        "plan": plan.token,
        "wcl": plan.wcl,
        "added": plan.added,
        "removed": plan.removed,
        "renamed": plan.renamed.iter()
            .map(|(from, to)| json!({ "from": from, "to": to }))
            .collect::<Vec<_>>(),
        "changes": changes,
        "flagged": plan.flagged,
        "retemplated": plan.retemplated,
    })
}
