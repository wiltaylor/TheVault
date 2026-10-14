//! The sidebar's pinned tree: pinned notes and folders, each of which can hold more of both. Pins
//! live in `state.db`, so every device sees the same ones.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{PinEntry, PinKind, PinNode};
use writer::Writer;

use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

/// `GET /api/pins`: the pinned tree. Each item is `{kind: "note", id, type, title, children}` or
/// `{kind: "folder", id, name, children}`. A pinned note in the trash is left out until it is
/// restored, and its children are shown in its place.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let pins = blocking(w, |w| {
        let mut notes = HashMap::new();
        for e in w.state().pin_entries()? {
            if e.kind == PinKind::Note
                && let Some(found) = w.type_and_title(&e.id)?
            {
                notes.insert(e.id, found);
            }
        }
        let tree = w.state().pin_tree(|id| Ok(notes.contains_key(id)))?;
        Ok::<_, ApiError>(
            tree.iter()
                .map(|n| node_json(n, &notes))
                .collect::<Vec<_>>(),
        )
    })
    .await?;
    Ok(Json(json!({ "pins": pins })))
}

fn node_json(n: &PinNode, notes: &HashMap<String, (String, String)>) -> Value {
    let children: Vec<Value> = n.children.iter().map(|c| node_json(c, notes)).collect();
    match &n.kind {
        PinKind::Folder { name } => {
            json!({ "kind": "folder", "id": n.id, "name": name, "children": children })
        }
        PinKind::Note => {
            let (type_name, title) = notes.get(&n.id).cloned().unwrap_or_default();
            json!({ "kind": "note", "id": n.id, "type": type_name, "title": title, "children": children })
        }
    }
}

fn entry_json(e: &PinEntry) -> Value {
    let mut out = json!({ "id": e.id, "parent": e.parent, "index": e.position });
    match &e.kind {
        PinKind::Folder { name } => {
            out["kind"] = json!("folder");
            out["name"] = json!(name);
        }
        PinKind::Note => out["kind"] = json!("note"),
    }
    out
}

/// `PUT /api/pins/{id}`: pin a note to the end of the list. Pinning a pinned note changes nothing.
pub async fn pin(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let note = blocking(w, move |w| {
        let Some((type_name, title)) = w.type_and_title(&id)? else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("there is no note {id}"),
            ));
        };
        let added = w.state().pin(&id)?;
        Ok(json!({ "kind": "note", "id": id, "type": type_name, "title": title, "added": added }))
    })
    .await?;
    Ok(Json(note))
}

/// `DELETE /api/pins/{id}`: unpin a note; its children move up into its place. Unpinning a note that is not pinned is a 404.
pub async fn unpin(State(w): Shared, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    blocking(w, move |w| {
        if w.state().unpin(&id)? {
            Ok(StatusCode::NO_CONTENT)
        } else {
            Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("note {id} is not pinned"),
            ))
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderBody {
    name: String,
    #[serde(default)]
    parent: Option<String>,
}

/// `POST /api/pins/folders`: create a folder at the end of `parent`'s children, or of the root.
pub async fn create_folder(
    State(w): Shared,
    Json(body): Json<FolderBody>,
) -> Result<Response, ApiError> {
    let folder = blocking(w, move |w| {
        w.state()
            .create_pin_folder(&body.name, body.parent.as_deref())
    })
    .await?;
    let mut out = entry_json(&folder);
    out["children"] = json!([]);
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameBody {
    name: String,
}

/// `PUT /api/pins/folders/{id}`: rename a folder.
pub async fn rename_folder(
    State(w): Shared,
    Path(id): Path<String>,
    Json(body): Json<RenameBody>,
) -> Result<Json<Value>, ApiError> {
    let folder = blocking(w, move |w| w.state().rename_pin_folder(&id, &body.name)).await?;
    Ok(Json(entry_json(&folder)))
}

/// `DELETE /api/pins/folders/{id}`: delete a folder; its children move up into its place.
pub async fn delete_folder(
    State(w): Shared,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    blocking(w, move |w| w.state().delete_pin_folder(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveBody {
    id: String,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    index: Option<usize>,
}

/// `POST /api/pins/move`: move a pinned note or folder to `parent` (`null` for the root) at
/// `index` among its children, counted without the moved entry and clamped to the end. Moving an
/// entry into itself or its own descendant is a 409.
pub async fn move_entry(
    State(w): Shared,
    Json(body): Json<MoveBody>,
) -> Result<Json<Value>, ApiError> {
    let moved = blocking(w, move |w| {
        w.state().move_pin(
            &body.id,
            body.parent.as_deref(),
            body.index.unwrap_or(usize::MAX),
        )
    })
    .await?;
    Ok(Json(entry_json(&moved)))
}
