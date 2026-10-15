//! Error responses: a status and a JSON body with a stable `error` code and a `message`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use writer::{Error, Writer};

/// An error response.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: Value,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            body: json!({ "error": code, "message": message.into() }),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl ApiError {
    fn from_state(e: state::Error) -> ApiError {
        use state::Error as S;
        let message = e.to_string();
        let (status, code) = match &e {
            S::TokenExists(_) => (StatusCode::CONFLICT, "token_exists"),
            S::UnknownToken(_)
            | S::UnknownSecret(_)
            | S::UnknownNote(_)
            | S::NotInTrash(_)
            | S::UnknownPin(_)
            | S::UnknownNotification(_)
            | S::UnknownDevice(_)
            | S::UnknownCommand(_) => (StatusCode::NOT_FOUND, "not_found"),
            S::CommandClosed { .. } => (StatusCode::CONFLICT, "command_closed"),
            S::InvalidCommandReport(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            S::InvalidNotification(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            S::PinCycle { .. } => (StatusCode::CONFLICT, "pin_cycle"),
            S::EmptyFolderName => (StatusCode::BAD_REQUEST, "bad_request"),
            S::UnknownScope(_) | S::UnknownPlatform(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            S::UnknownVersion { .. } => (StatusCode::BAD_REQUEST, "unknown_version"),
            S::KeyMissing { .. } | S::WrongKey { .. } => {
                (StatusCode::SERVICE_UNAVAILABLE, "secret_key")
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        ApiError::new(status, code, message)
    }
}

impl From<state::Error> for ApiError {
    fn from(e: state::Error) -> ApiError {
        ApiError::from_state(e)
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> ApiError {
        let message = e.to_string();
        let (status, body) = match e {
            Error::Parse { file, .. } => (
                StatusCode::BAD_REQUEST,
                json!({ "error": "parse", "file": file }),
            ),
            Error::DataOnly(d) => (
                StatusCode::BAD_REQUEST,
                json!({ "error": "not_data_only", "file": d.file, "line": d.line, "item": d.item }),
            ),
            Error::BadJson(_) => (StatusCode::BAD_REQUEST, json!({ "error": "bad_json" })),
            Error::Invalid(inv) => (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "invalid",
                    "file": inv.file,
                    "issues": inv.issues.iter()
                        .map(|i| json!({ "at": i.at, "message": i.message }))
                        .collect::<Vec<_>>(),
                }),
            ),
            Error::TypeHasNotes { type_name, notes } => (
                StatusCode::CONFLICT,
                json!({
                    "error": "type_in_use",
                    "type": type_name,
                    "count": notes.len(),
                    "notes": notes,
                }),
            ),
            Error::PlanStale(plan) => (
                StatusCode::CONFLICT,
                json!({ "error": "plan_stale", "plan": crate::types::plan_json(&plan, false) }),
            ),
            Error::UnknownType(t) => (
                StatusCode::BAD_REQUEST,
                json!({ "error": "unknown_type", "type": t }),
            ),
            Error::NotFound(_) | Error::VersionNotFound { .. } | Error::NotInTrash(_) => {
                (StatusCode::NOT_FOUND, json!({ "error": "not_found" }))
            }
            Error::BaseRequired(_) => (
                StatusCode::PRECONDITION_REQUIRED,
                json!({ "error": "base_version_required" }),
            ),
            Error::Conflict {
                id,
                current_version,
                current,
            } => (
                StatusCode::CONFLICT,
                json!({
                    "error": "conflict",
                    "id": id,
                    "current_version": current_version,
                    "current": current,
                }),
            ),
            Error::Restricted { id, referrers } => (
                StatusCode::CONFLICT,
                json!({
                    "error": "restricted",
                    "id": id,
                    "referrers": referrers.iter()
                        .map(|r| json!({ "id": r.id, "type": r.type_name, "field": r.field }))
                        .collect::<Vec<_>>(),
                }),
            ),
            Error::Rejected(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "rejected" }),
            ),
            Error::Migration { id, .. } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "migration_failed", "id": id }),
            ),
            Error::InvalidType(e) => (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "invalid_type",
                    "failures": e.0.iter().map(|inv| json!({
                        "file": inv.file,
                        "issues": inv.issues.iter()
                            .map(|i| json!({ "at": i.at, "message": i.message }))
                            .collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                }),
            ),
            Error::BuiltInType(t) => (
                StatusCode::CONFLICT,
                json!({ "error": "built_in_type", "type": t }),
            ),
            Error::TypeInUse(t) => (
                StatusCode::CONFLICT,
                json!({ "error": "type_in_use", "type": t }),
            ),
            Error::BadRequest(_) => (StatusCode::BAD_REQUEST, json!({ "error": "bad_request" })),
            Error::Missing(_) => (StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
            Error::KeyNotUnique {
                type_name,
                key,
                ids,
            } => (
                StatusCode::CONFLICT,
                json!({ "error": "key_not_unique", "type": type_name, "key": key, "ids": ids }),
            ),
            Error::InUse { code, by, .. } => {
                (StatusCode::CONFLICT, json!({ "error": code, "by": by }))
            }
            Error::State(e) => return ApiError::from_state(e),
            Error::Schema(_) | Error::Store(_) | Error::Index(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "internal" }),
            ),
        };
        let mut body = body;
        body["message"] = Value::String(message);
        ApiError { status, body }
    }
}

/// `e` as a response. A refusal because something is in use also lists, under `notes`, each note
/// among its users with the note's type and title, so a client can show what to change.
pub fn in_use(w: &Writer, e: Error) -> ApiError {
    let paths: Vec<String> = match &e {
        Error::InUse { by, .. } => by.clone(),
        _ => return e.into(),
    };
    let mut err = ApiError::from(e);
    let mut notes: Vec<Value> = Vec::new();
    for path in &paths {
        let Some((type_name, id)) = path
            .strip_prefix("notes/")
            .and_then(|rest| rest.split_once(".wcl"))
            .and_then(|(rest, _)| rest.split_once('/'))
        else {
            continue;
        };
        if notes.iter().any(|n| n["id"] == id) {
            continue;
        }
        let title = w.title(id).ok().flatten().unwrap_or_default();
        notes.push(json!({ "id": id, "type": type_name, "title": title }));
    }
    err.body["notes"] = Value::Array(notes);
    err
}
