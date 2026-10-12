//! The HTTP API: routes, password session and token auth, scope checks and conflict responses.
//!
//! Notes are read and written as WCL text or JSON (D32). A request whose `Content-Type` is
//! `application/json` sends `{ "type"?, "base_version"?, "note" | "wcl" }`, where `note` is the
//! JSON form of [`writer::Input::Json`]; any other body is the note's WCL text, with the base
//! version in `If-Match`. A response is JSON unless `Accept` asks for `text/plain` or WCL, in
//! which case the body is the canonical WCL on disk and the version is in `ETag` (D21).

mod auth;
mod error;
mod notes;
mod types;

use std::sync::Arc;

use axum::{
    Json, Router,
    routing::{get, post, put},
};
use serde_json::{Value, json};
use writer::Writer;

pub use auth::Caller;
pub use error::ApiError;

/// Builds the router for every `/api` route.
pub fn router(writer: Arc<Writer>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/notes", get(notes::list).post(notes::create))
        .route(
            "/api/notes/{id}",
            get(notes::read).put(notes::update).delete(notes::delete),
        )
        .route("/api/notes/{id}/history", get(notes::history))
        .route("/api/notes/{id}/history/{version}", get(notes::version))
        .route("/api/notes/{id}/restore", post(notes::restore_version))
        .route("/api/trash", get(notes::trash))
        .route("/api/trash/{id}/restore", post(notes::restore_from_trash))
        .route("/api/types", post(types::create))
        .route(
            "/api/types/{name}",
            put(types::change).delete(types::delete),
        )
        .route("/api/flagged", get(types::flagged))
        .with_state(writer)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
