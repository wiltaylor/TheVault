//! Type definitions. Creating a type, or replacing one that has no notes, writes its file at once;
//! changing a type that has notes needs a planned schema change (D6).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use writer::Writer;

use crate::notes::{is_json, wants_wcl};
use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

fn type_response(headers: &HeaderMap, name: &str, wcl: String) -> Response {
    if wants_wcl(headers) {
        let mut res = wcl.into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        return res;
    }
    Json(json!({ "name": name, "wcl": wcl })).into_response()
}

/// `GET /api/types/{name}`: the type's definition as stored.
pub async fn read(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let n = name.clone();
    let wcl = blocking(w, move |w| w.read_type(&n)).await?;
    Ok(type_response(&headers, &name, wcl))
}

/// `PUT /api/types/{name}`: create or replace a type from its WCL, sent as text or as
/// `{ "wcl": "..." }`.
pub async fn put(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let source = if is_json(&headers) {
        let value: Value = serde_json::from_slice(&body)
            .map_err(|e| ApiError::bad_request(format!("the body is not JSON: {e}")))?;
        match value.as_object().map(|o| (o.len(), o.get("wcl"))) {
            Some((1, Some(Value::String(s)))) => s.clone(),
            _ => return Err(ApiError::bad_request("send `{ \"wcl\": \"...\" }`")),
        }
    } else {
        String::from_utf8(body.to_vec())
            .map_err(|_| ApiError::bad_request("the WCL body is not UTF-8"))?
    };
    let n = name.clone();
    let wcl = blocking(w, move |w| w.put_type(&n, &source)).await?;
    Ok(type_response(&headers, &name, wcl))
}
