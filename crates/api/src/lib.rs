//! The HTTP API: routes, password session and token auth, scope checks and conflict responses.
//!
//! Notes are read and written as WCL text or JSON (D32). A request whose `Content-Type` is
//! `application/json` sends `{ "type"?, "base_version"?, "note" | "wcl" }`, where `note` is the
//! JSON form of [`writer::Input::Json`]; any other body is the note's WCL text, with the base
//! version in `If-Match`. A response is JSON unless `Accept` asks for `text/plain` or WCL, in
//! which case the body is the canonical WCL on disk and the version is in `ETag` (D21).
//!
//! Queries run at `/api/query` and full-text search at `/api/search`, a page at a time with a
//! cursor (D33). `/api/query/parse` and `/api/query/text` convert between text and the visual
//! builder's model (D8).
//!
//! Every route but health and login needs a session cookie or a bearer token, and the scope
//! [`auth::required_scope`] names for it (D11).

mod auth;
mod error;
mod notes;
mod query;
mod tokens;
mod types;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    Json, Router, middleware,
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use writer::Writer;

pub use auth::{Caller, OWNER_SESSION, SESSION_COOKIE, required_scope};
pub use error::ApiError;

/// Builds the router for every `/api` route.
pub fn router(writer: Arc<Writer>) -> Router {
    let sessions = Arc::new(auth::Sessions::new(writer.clone()));
    let login = Router::new()
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .with_state(sessions.clone());
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
        .route("/api/query", get(query::run_get).post(query::run_post))
        .route("/api/query/parse", post(query::parse_query))
        .route("/api/query/text", post(query::model_text))
        .route("/api/search", get(query::search))
        .route("/api/trash", get(notes::trash))
        .route("/api/trash/{id}/restore", post(notes::restore_from_trash))
        .route(
            "/api/types/{name}",
            get(types::read).put(types::put).delete(types::delete),
        )
        .route("/api/types/{name}/change", post(types::change))
        .route("/api/flagged", get(types::flagged))
        .route("/api/tokens", get(tokens::list).post(tokens::create))
        .route("/api/tokens/{name}", delete(tokens::revoke))
        .with_state(writer)
        .merge(login)
        .layer(middleware::from_fn_with_state(sessions, auth::authenticate))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Run blocking work against the writer off the async runtime.
pub(crate) async fn blocking<T, E>(
    writer: Arc<Writer>,
    f: impl FnOnce(&Writer) -> Result<T, E> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    E: Into<ApiError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || f(&writer))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(Into::into)
}
