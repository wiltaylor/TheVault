//! Error responses: a status and a JSON body with a stable `error` code and a `message`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use writer::Error;

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
            S::UnknownToken(_) | S::UnknownSecret(_) | S::UnknownNote(_) | S::NotInTrash(_) => {
                (StatusCode::NOT_FOUND, "not_found")
            }
            S::UnknownScope(_) => (StatusCode::BAD_REQUEST, "bad_request"),
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
                json!({ "error": "type_in_use", "type": type_name, "notes": notes }),
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
            Error::TypeInUse(t) => (
                StatusCode::CONFLICT,
                json!({ "error": "type_in_use", "type": t }),
            ),
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
