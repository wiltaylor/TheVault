//! Type create, change and delete, and the flagged-note listing (D6).
//!
//! These change the schema, so they need the `admin` scope once tokens land (D11).
//!
//! - `POST /api/types` takes `{ "name", "wcl" }` and creates `types/<name>.wcl`.
//! - `PUT /api/types/{name}` takes `{ "wcl", "renames"?, "plan"? }`. Without `plan` it returns the
//!   plan and changes nothing. With the `plan` token from that response it applies the change as
//!   one commit, or answers 409 `plan_stale` with the plan as it is now.
//! - `DELETE /api/types/{name}` deletes a type that has no notes.
//! - `GET /api/flagged` lists notes holding data their type no longer describes.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{NoteEdit, TypeChange, TypePlan, Writer};

use crate::notes::blocking;
use crate::{ApiError, Caller};

type Shared = State<Arc<Writer>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    name: String,
    wcl: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeBody {
    wcl: String,
    #[serde(default)]
    renames: BTreeMap<String, String>,
    plan: Option<String>,
}

pub async fn create(State(w): Shared, Json(body): Json<CreateBody>) -> Result<Response, ApiError> {
    let name = body.name.clone();
    let wcl = blocking(w, move |w| w.create_type(&body.name, &body.wcl)).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": name, "wcl": wcl })),
    )
        .into_response())
}

pub async fn change(
    State(w): Shared,
    Caller(author): Caller,
    Path(name): Path<String>,
    Json(body): Json<ChangeBody>,
) -> Result<Json<Value>, ApiError> {
    let change = TypeChange {
        wcl: body.wcl,
        renames: body.renames,
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

pub async fn delete(State(w): Shared, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let name2 = name.clone();
    blocking(w, move |w| w.delete_type(&name2))
        .await
        .map_err(missing_type)?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

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
    })
}
