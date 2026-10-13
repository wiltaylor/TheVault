//! Notes pinned to the sidebar. Pins live in `state.db`, so every device sees the same ones.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};
use writer::Writer;

use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

/// `GET /api/pins`: the pinned notes in sidebar order, each with its type and title. A pinned note
/// that is in the trash is left out until it is restored.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let pins = blocking(w, |w| {
        let mut out = Vec::new();
        for id in w.state().pins()? {
            if let Some((type_name, title)) = w.type_and_title(&id)? {
                out.push(json!({ "id": id, "type": type_name, "title": title }));
            }
        }
        Ok::<_, ApiError>(out)
    })
    .await?;
    Ok(Json(json!({ "pins": pins })))
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
        Ok(json!({ "id": id, "type": type_name, "title": title, "added": added }))
    })
    .await?;
    Ok(Json(note))
}

/// `DELETE /api/pins/{id}`: unpin a note. Unpinning a note that is not pinned is a 404.
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
