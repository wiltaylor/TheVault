//! The endpoint listing: an OpenAPI 3.1 document of every route, served at
//! `GET /api/openapi.json`.
//!
//! [`ENDPOINTS`] is written by hand, separately from the router. A test compares it with the routes
//! the router registers, so a route added without a listing entry, or a listing entry left behind
//! by a removed route, fails the build.

use axum::Json;
use axum::http::Method;
use serde_json::{Map, Value, json};

use crate::required_scope;

/// Every endpoint: method, path and a one-line summary.
pub const ENDPOINTS: &[(&str, &str, &str)] = &[
    ("GET", "/api/health", "Report that the server is up"),
    ("GET", "/api/openapi.json", "This listing of every endpoint"),
    (
        "POST",
        "/api/login",
        "Log in with the vault password and get a session cookie",
    ),
    ("POST", "/api/logout", "End the session"),
    (
        "GET",
        "/api/schema",
        "Every type with its fields, kinds, relationships, templates and handlers, and every taxonomy",
    ),
    ("GET", "/api/notes", "List notes, optionally of one ?type="),
    ("POST", "/api/notes", "Create a note from WCL or JSON"),
    ("GET", "/api/notes/{id}", "Read a note as JSON or WCL"),
    (
        "PUT",
        "/api/notes/{id}",
        "Replace a note at its base version",
    ),
    (
        "DELETE",
        "/api/notes/{id}",
        "Delete a note at its base version, moving it to the trash",
    ),
    ("GET", "/api/notes/{id}/history", "Every version of a note"),
    (
        "GET",
        "/api/notes/{id}/history/{version}",
        "One version of a note",
    ),
    (
        "POST",
        "/api/notes/{id}/restore",
        "Write an earlier version of a note back as a new version",
    ),
    (
        "GET",
        "/api/notes/{id}/canvas/{name}",
        "Read one canvas of a note",
    ),
    (
        "PUT",
        "/api/notes/{id}/canvas/{name}",
        "Replace one canvas of a note at its base version",
    ),
    (
        "POST",
        "/api/batch",
        "Apply a batch of creates, updates, deletes and upserts in one commit, or dry-run it",
    ),
    (
        "POST",
        "/api/upsert",
        "Create or update the note matching a key field",
    ),
    ("GET", "/api/query", "Run a query, a page at a time"),
    (
        "POST",
        "/api/query",
        "Run a query sent in the body, a page at a time",
    ),
    (
        "POST",
        "/api/query/parse",
        "Turn query text into the visual builder's model",
    ),
    (
        "POST",
        "/api/query/text",
        "Turn a builder model into query text",
    ),
    ("GET", "/api/search", "Full-text search, a page at a time"),
    ("GET", "/api/trash", "Every note in the trash"),
    (
        "POST",
        "/api/trash/{id}/restore",
        "Bring a note back from the trash",
    ),
    (
        "GET",
        "/api/types",
        "Every type with its fields and templates",
    ),
    ("GET", "/api/types/{name}", "Read a type definition"),
    (
        "DELETE",
        "/api/types/{name}",
        "Delete a type that has no notes",
    ),
    (
        "POST",
        "/api/types/{name}/change",
        "Plan a schema change, or apply a plan by its token",
    ),
    (
        "GET",
        "/api/flagged",
        "Notes holding data their type no longer describes",
    ),
    (
        "PUT",
        "/api/types/{name}",
        "Create a type, or replace one that has no notes",
    ),
    (
        "GET",
        "/api/types/{name}/templates",
        "Every template of a type",
    ),
    (
        "GET",
        "/api/types/{name}/templates/{template}",
        "Read one template block",
    ),
    (
        "PUT",
        "/api/types/{name}/templates/{template}",
        "Create or replace one template",
    ),
    (
        "DELETE",
        "/api/types/{name}/templates/{template}",
        "Delete a template other than default",
    ),
    (
        "GET",
        "/api/taxonomies",
        "Every taxonomy with its term tree",
    ),
    ("GET", "/api/taxonomies/{name}", "Read a taxonomy"),
    (
        "PUT",
        "/api/taxonomies/{name}",
        "Create or replace a taxonomy, renaming or moving terms in every note",
    ),
    (
        "DELETE",
        "/api/taxonomies/{name}",
        "Delete a taxonomy no field uses",
    ),
    ("GET", "/api/scripts", "Every script name"),
    ("GET", "/api/scripts/{name}", "Read a script's source"),
    ("PUT", "/api/scripts/{name}", "Create or replace a script"),
    (
        "DELETE",
        "/api/scripts/{name}",
        "Delete a script no handler or task names",
    ),
    ("GET", "/api/assets", "Every uploaded asset"),
    (
        "POST",
        "/api/assets",
        "Upload a file as a new asset, named with ?filename=",
    ),
    ("GET", "/api/assets/{id}", "Download an asset's bytes"),
    (
        "DELETE",
        "/api/assets/{id}",
        "Delete an asset; ?unlink=true first takes it out of every note that names it",
    ),
    (
        "GET",
        "/api/daily/{date}",
        "The daily note for a YYYY-MM-DD day; 404 when the day has none",
    ),
    (
        "POST",
        "/api/daily/{date}",
        "The daily note for a YYYY-MM-DD day, created when the day has none",
    ),
    (
        "GET",
        "/api/pins",
        "The sidebar's pinned tree: notes with type and title, and folders, each with children",
    ),
    (
        "PUT",
        "/api/pins/{id}",
        "Pin a note to the end of the sidebar's root",
    ),
    (
        "DELETE",
        "/api/pins/{id}",
        "Unpin a note; its children move up into its place",
    ),
    (
        "POST",
        "/api/pins/folders",
        "Create a pinned folder from {name, parent?}",
    ),
    ("PUT", "/api/pins/folders/{id}", "Rename a pinned folder"),
    (
        "DELETE",
        "/api/pins/folders/{id}",
        "Delete a pinned folder; its children move up into its place",
    ),
    (
        "POST",
        "/api/pins/move",
        "Move a pinned note or folder: {id, parent, index}; a move into itself is a 409",
    ),
    ("GET", "/api/tokens", "Every API token, without secrets"),
    ("POST", "/api/tokens", "Create an API token with one scope"),
    ("DELETE", "/api/tokens/{name}", "Revoke an API token"),
    (
        "GET",
        "/api/secrets",
        "Every secret's name and times, never a value",
    ),
    (
        "GET",
        "/api/secrets/{name}",
        "A secret's name and times, never its value",
    ),
    (
        "PUT",
        "/api/secrets/{name}",
        "Store or replace a secret, encrypted at rest",
    ),
    ("DELETE", "/api/secrets/{name}", "Delete a secret"),
    (
        "GET",
        "/api/tasks",
        "Every scheduled task with its next run and latest run",
    ),
    (
        "GET",
        "/api/tasks/{name}",
        "Read a scheduled task's file and status",
    ),
    (
        "PUT",
        "/api/tasks/{name}",
        "Create or replace a scheduled task",
    ),
    ("DELETE", "/api/tasks/{name}", "Delete a scheduled task"),
    (
        "GET",
        "/api/tasks/{name}/runs",
        "A task's runs, newest first, with their logs",
    ),
    ("GET", "/api/tasks/{name}/runs/{id}", "One run of a task"),
    (
        "POST",
        "/api/tasks/{name}/run",
        "Run a task now and wait for the run to end",
    ),
];

/// The OpenAPI document of [`ENDPOINTS`].
pub fn document() -> Value {
    let mut paths = Map::new();
    for (method, path, summary) in ENDPOINTS {
        let m = Method::from_bytes(method.as_bytes()).expect("a listed method is valid");
        let params: Vec<Value> = path
            .split('/')
            .filter_map(|seg| seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
            .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        let mut op = json!({
            "summary": summary,
            "operationId": operation_id(method, path),
            "responses": { "default": { "description": "JSON, or WCL text when Accept asks for it" } },
        });
        match required_scope(&m, path) {
            Some(scope) => {
                op["x-scope"] = json!(scope.as_str());
                op["security"] = json!([{ "bearer": [] }, { "session": [] }]);
            }
            None => op["security"] = json!([]),
        }
        if !params.is_empty() {
            op["parameters"] = Value::Array(params);
        }
        let entry = paths
            .entry(path.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        entry[method.to_ascii_lowercase()] = op;
    }
    json!({
        "openapi": "3.1.0",
        "info": { "title": "TheVault API", "version": env!("CARGO_PKG_VERSION") },
        "paths": paths,
        "components": {
            "securitySchemes": {
                "bearer": { "type": "http", "scheme": "bearer" },
                "session": { "type": "apiKey", "in": "cookie", "name": crate::SESSION_COOKIE },
            }
        },
    })
}

/// `get_notes_id_history`, from `GET /api/notes/{id}/history`.
fn operation_id(method: &str, path: &str) -> String {
    let mut id = method.to_ascii_lowercase();
    for seg in path.trim_start_matches("/api/").split('/') {
        let seg: String = seg
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let seg = seg.trim_matches('_');
        if !seg.is_empty() {
            id.push('_');
            id.push_str(seg);
        }
    }
    id
}

/// `GET /api/openapi.json`.
pub async fn serve() -> Json<Value> {
    Json(document())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_listing_matches_the_registered_routes() {
        let listed: BTreeSet<(String, String)> = ENDPOINTS
            .iter()
            .map(|(m, p, _)| (m.to_string(), p.to_string()))
            .collect();
        assert_eq!(listed.len(), ENDPOINTS.len(), "an endpoint is listed twice");
        let registered: BTreeSet<(String, String)> = crate::routes()
            .into_iter()
            .map(|(m, p)| (m.to_string(), p.to_owned()))
            .collect();
        let unlisted: Vec<_> = registered.difference(&listed).collect();
        let stale: Vec<_> = listed.difference(&registered).collect();
        assert!(
            unlisted.is_empty(),
            "registered but not listed: {unlisted:?}"
        );
        assert!(stale.is_empty(), "listed but not registered: {stale:?}");
    }

    #[test]
    fn operation_ids_are_unique() {
        let ids: BTreeSet<String> = ENDPOINTS
            .iter()
            .map(|(m, p, _)| operation_id(m, p))
            .collect();
        assert_eq!(ids.len(), ENDPOINTS.len());
        assert_eq!(
            operation_id("GET", "/api/notes/{id}/history"),
            "get_notes_id_history"
        );
    }
}
