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
    let mut out = json!({
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
    });
    with_list_page(&mut out, t);
    with_extra(&mut out, &t.extra);
    out
}

/// Add `list_page` to `out` when the type file has the block: each item the block holds, the
/// sort as the file writes it, such as `"sent desc"` (lists L1). The client applies the defaults.
pub(crate) fn with_list_page(out: &mut Value, t: &TypeDef) {
    let Some(lp) = &t.list_page else {
        return;
    };
    let mut m = Map::new();
    if let Some(v) = &lp.view {
        m.insert("view".into(), json!(v));
    }
    if let Some(s) = lp.sort_text() {
        m.insert("sort".into(), json!(s));
    }
    if let Some(n) = lp.page_size {
        m.insert("page_size".into(), json!(n));
    }
    out["list_page"] = Value::Object(m);
}

/// Add `extra` to `out` when there is any: each attribute this version does not know, by name,
/// as WCL source text, so an editor that rewrites the definition can write it back as it was.
pub(crate) fn with_extra(out: &mut Value, extra: &[(String, schema::Value)]) {
    if extra.is_empty() {
        return;
    }
    let map: Map<String, Value> = extra
        .iter()
        .map(|(n, v)| (n.clone(), json!(writer::value_wcl(v))))
        .collect();
    out["extra"] = Value::Object(map);
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
        FieldKind::Select {
            options,
            option_labels,
        } => {
            out.insert("options".into(), json!(options));
            if !option_labels.is_empty() {
                out.insert("option_labels".into(), json!(option_labels));
            }
        }
        FieldKind::Taxonomy { taxonomy } => {
            out.insert("taxonomy".into(), json!(taxonomy));
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
        FieldKind::List { item } => {
            out.insert("items".into(), json!(item.kind.name()));
            out.insert("item".into(), field_json(item));
        }
        FieldKind::Asset => {
            out.insert("accept".into(), json!(f.accept.name()));
        }
        FieldKind::Dict { keys, values } => {
            out.insert(
                "keys".into(),
                json!(keys.iter().map(field_json).collect::<Vec<_>>()),
            );
            out.insert(
                "values".into(),
                match values {
                    schema::DictValues::None => Value::Null,
                    schema::DictValues::Any => json!("any"),
                    schema::DictValues::Of(d) => field_json(d),
                },
            );
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
    // Declared as a list of selects, terms, references or assets: that kind holding several.
    if f.listed {
        out.insert("listed".into(), json!(true));
        out.insert("items".into(), json!(f.kind.name()));
    }
    let mut out = Value::Object(out);
    with_extra(&mut out, &f.extra);
    out
}

pub(crate) fn template_json(t: &Template) -> Value {
    let mut out = json!({
        "name": t.name,
        "label": t.label,
        "tabs": t.tabs.iter().map(|tab| {
            let mut out = json!({
                "title": tab.title,
                "items": tab.items.iter().map(|item| match item {
                    TabItem::Slot { field, label, when, extra } => {
                        let mut out = json!({ "slot": field, "label": label });
                        // Only a conditional slot carries `when`, as only a conditional tab does.
                        if let Some(w) = when {
                            out["when"] = json!(w);
                        }
                        with_extra(&mut out, extra);
                        out
                    }
                    TabItem::Block(b) => json!({ "block": block_to_json(b) }),
                }).collect::<Vec<_>>(),
            });
            // Only a conditional tab carries `when`, so the JSON of every other type is unchanged.
            if let Some(w) = &tab.when {
                out["when"] = json!(w);
            }
            with_extra(&mut out, &tab.extra);
            out
        }).collect::<Vec<_>>(),
    });
    with_extra(&mut out, &t.extra);
    out
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
