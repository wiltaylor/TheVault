//! The HTTP API: routes, password session and token auth, scope checks and conflict responses.

use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

/// Builds the router for every `/api` route.
pub fn router() -> Router {
    Router::new().route("/api/health", get(health))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
