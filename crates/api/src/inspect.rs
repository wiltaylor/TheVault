//! Schema inspection (D32): `GET /api/schema` returns every type with its fields, kinds,
//! relationships, templates and handlers, and every taxonomy with its terms.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use schema::{FieldDef, FieldKind, OnDelete, Relation, TabItem, Taxonomy, Template, Term, TypeDef};
use serde_json::{Map, Value, json};
use writer::{Writer, block_to_json, value_to_json};

use crate::{ApiError, blocking};

/// `GET /api/schema`.
pub async fn schema(State(w): State<Arc<Writer>>) -> Result<Json<Value>, ApiError> {
    let (schema, builtin) = blocking(w, |w| {
        let schema = w.schema();
        let builtin: Vec<bool> = schema.types().map(|t| w.is_built_in(&t.name)).collect();
        Ok::<_, writer::Error>((schema, builtin))
    })
    .await?;
    Ok(Json(json!({
        "types": schema
            .types()
            .zip(builtin)
            .map(|(t, b)| type_json(t, b))
            .collect::<Vec<_>>(),
        "taxonomies": schema.taxonomies().map(taxonomy_json).collect::<Vec<_>>(),
    })))
}

pub(crate) fn type_json(t: &TypeDef, builtin: bool) -> Value {
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
        "handlers": t.handlers.iter().map(|h| json!({
            "event": h.event,
            "script": h.script,
            "function": h.function,
            "allow_domains": h.allow_domains,
            "secrets": h.secrets,
        })).collect::<Vec<_>>(),
        "warnings": t.warnings.iter().map(crate::error::issue_json).collect::<Vec<_>>(),
    })
}

fn field_json(f: &FieldDef) -> Value {
    let mut out = Map::new();
    out.insert("name".into(), json!(f.name));
    out.insert("kind".into(), json!(f.kind.name()));
    out.insert("label".into(), json!(f.label));
    out.insert("description".into(), json!(f.description));
    out.insert("required".into(), json!(f.required));
    out.insert("computed".into(), json!(f.computed));
    out.insert("hidden".into(), json!(f.hidden));
    out.insert("multiple".into(), json!(f.multiple));
    out.insert(
        "default".into(),
        f.default.as_ref().map_or(Value::Null, value_to_json),
    );
    match &f.kind {
        FieldKind::DateRange { of } => {
            out.insert("of".into(), json!(of.name()));
        }
        FieldKind::Select { options } => {
            out.insert("options".into(), json!(options));
        }
        FieldKind::Taxonomy { taxonomy } => {
            out.insert("taxonomy".into(), json!(taxonomy));
            if f.term_list {
                // Declared as a list of terms; it reads as a multiple taxonomy field.
                out.insert("items".into(), json!("taxonomy"));
            }
        }
        FieldKind::Reference(r) => {
            out.insert("target".into(), json!(r.target.as_deref().unwrap_or("*")));
            out.insert(
                "relation".into(),
                json!(match r.relation {
                    Relation::OneWay => "one_way",
                    Relation::TwoWay => "two_way",
                    Relation::Peer => "peer",
                }),
            );
            out.insert("inverse".into(), json!(r.inverse));
            out.insert(
                "on_delete".into(),
                json!(match r.on_delete {
                    OnDelete::Unlink => "unlink",
                    OnDelete::Restrict => "restrict",
                    OnDelete::Cascade => "cascade",
                }),
            );
        }
        FieldKind::List { items } => {
            out.insert("items".into(), json!(scalar_name(*items)));
            out.insert("labels".into(), json!(f.labels));
        }
        FieldKind::Asset => {
            out.insert("accept".into(), json!(f.accept.name()));
        }
        FieldKind::Dict { values } => {
            out.insert("values".into(), json!(scalar_name(*values)));
        }
        FieldKind::Location { image } => {
            out.insert(
                "map".into(),
                json!(if image.is_some() { "image" } else { "geo" }),
            );
            out.insert("image".into(), json!(image));
        }
        _ => {}
    }
    Value::Object(out)
}

fn scalar_name(s: schema::Scalar) -> &'static str {
    use schema::Scalar as S;
    match s {
        S::String => "string",
        S::Int => "int",
        S::Float => "float",
        S::Bool => "bool",
        S::Date => "date",
        S::Datetime => "datetime",
        S::Url => "url",
    }
}

pub(crate) fn template_json(t: &Template) -> Value {
    json!({
        "name": t.name,
        "label": t.label,
        "tabs": t.tabs.iter().map(|tab| {
            let mut out = json!({
                "title": tab.title,
                "items": tab.items.iter().map(|item| match item {
                    TabItem::Slot { field, label } => json!({ "slot": field, "label": label }),
                    TabItem::Block(b) => json!({ "block": block_to_json(b) }),
                }).collect::<Vec<_>>(),
            });
            // Only a conditional tab carries `when`, so the JSON of every other type is unchanged.
            if let Some(w) = &tab.when {
                out["when"] = json!(w);
            }
            out
        }).collect::<Vec<_>>(),
    })
}

pub(crate) fn taxonomy_json(t: &Taxonomy) -> Value {
    fn terms(list: &[Term], prefix: &str) -> Vec<Value> {
        list.iter()
            .map(|t| {
                let path = if prefix.is_empty() {
                    t.name.clone()
                } else {
                    format!("{prefix}/{}", t.name)
                };
                json!({
                    "name": t.name,
                    "path": path,
                    "colour": t.colour,
                    "description": t.description,
                    "children": terms(&t.children, &path),
                })
            })
            .collect()
    }
    json!({
        "name": t.name,
        "label": t.label,
        "description": t.description,
        "allow_new": t.allow_new,
        "terms": terms(&t.terms, ""),
    })
}
