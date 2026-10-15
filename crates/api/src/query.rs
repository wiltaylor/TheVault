//! Queries, the visual builder's model and full-text search (D8, D16, D33).
//!
//! A result comes a page at a time: `page_size` rows, and a `next` cursor that fetches the page
//! after. A query that does not parse is a 400 with the line and column of the error.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use writer::Writer;

use crate::ApiError;

type Shared = State<Arc<Writer>>;

/// Rows per page when the request names no size.
pub const DEFAULT_PAGE: usize = 100;

/// A query to run, from a JSON body or URL parameters.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBody {
    query: String,
    /// `include` or `only` to read archived notes; see [`with_archived`].
    archived: Option<String>,
    /// The note that holds the query, bound to `this`.
    this: Option<String>,
    cursor: Option<String>,
    page_size: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    q: String,
    archived: Option<String>,
    this: Option<String>,
    cursor: Option<String>,
    page_size: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    q: String,
    /// `include` or `only` to search archived notes; they are left out otherwise.
    archived: Option<String>,
    cursor: Option<String>,
    page_size: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParseBody {
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBody {
    model: query::Model,
}

fn syntax_error(e: &query::ParseError) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        body: json!({
            "error": "query_syntax",
            "line": e.line,
            "column": e.column,
            "message": e.message,
        }),
    }
}

impl From<query::Error> for ApiError {
    fn from(e: query::Error) -> ApiError {
        let code = match &e {
            query::Error::NoThis => "no_this",
            query::Error::BadCursor => "bad_cursor",
            query::Error::Index(_) | query::Error::Sqlite(_) => {
                return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string());
            }
            _ => "query",
        };
        ApiError::new(StatusCode::BAD_REQUEST, code, e.to_string())
    }
}

fn json_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    body.map(|Json(b)| b)
        .map_err(|e| ApiError::bad_request(e.body_text()))
}

fn params<T>(q: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
    q.map(|Query(p)| p)
        .map_err(|e| ApiError::bad_request(e.body_text()))
}

/// Run one page of `q` off the async runtime.
async fn run_page(
    w: Arc<Writer>,
    q: query::Query,
    this: Option<String>,
    cursor: Option<String>,
    page_size: Option<usize>,
) -> Result<Json<Value>, ApiError> {
    let page = query::PageRequest {
        cursor,
        size: page_size.unwrap_or(DEFAULT_PAGE).min(query::MAX_PAGE),
    };
    let result = tokio::task::spawn_blocking(move || {
        w.read_index(|index, schema| {
            let mut ctx = query::Context::new(index, schema);
            ctx.this = this;
            query::run(&q, &ctx, &page)
        })
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))??;
    serde_json::to_value(result)
        .map(Json)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))
}

fn parse(text: &str) -> Result<query::Query, ApiError> {
    query::parse(text).map_err(|e| syntax_error(&e))
}

pub async fn run_post(
    State(w): Shared,
    body: Result<Json<RunBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let b = json_body(body)?;
    let q = with_archived(parse(&b.query)?, b.archived.as_deref())?;
    run_page(w, q, b.this, b.cursor, b.page_size).await
}

pub async fn run_get(
    State(w): Shared,
    p: Result<Query<RunParams>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let p = params(p)?;
    let q = with_archived(parse(&p.q)?, p.archived.as_deref())?;
    run_page(w, q, p.this, p.cursor, p.page_size).await
}

/// Full-text search over every note's fields and content, best match first.
pub async fn search(
    State(w): Shared,
    p: Result<Query<SearchParams>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let p = params(p)?;
    if p.q.trim().is_empty() {
        return Err(ApiError::bad_request("`q` is empty"));
    }
    let archived = archived_param(p.archived.as_deref())?;
    let q = query::Query::search_all(p.q, archived);
    run_page(w, q, None, p.cursor, p.page_size).await
}

/// `q` with an `archived` parameter applied: `include` adds `include archived`, `only` adds
/// `&& archived` to the filter, and `exclude` or none leaves the query to decide.
fn with_archived(mut q: query::Query, value: Option<&str>) -> Result<query::Query, ApiError> {
    match archived_param(value)? {
        writer::Archived::Exclude => {}
        writer::Archived::Include => q.include_archived = true,
        writer::Archived::Only => {
            let only = query::Expr::Name(query::ARCHIVED.to_owned());
            q.filter = Some(match q.filter.take() {
                Some(e) => query::Expr::Binary(query::BinOp::And, Box::new(e), Box::new(only)),
                None => only,
            });
        }
    }
    Ok(q)
}

/// An `archived` URL parameter: `exclude`, the default, `include` or `only`.
pub(crate) fn archived_param(value: Option<&str>) -> Result<writer::Archived, ApiError> {
    match value {
        None => Ok(writer::Archived::Exclude),
        Some(v) => writer::Archived::parse(v).ok_or_else(|| {
            ApiError::bad_request(format!(
                "`archived` is {v:?}: it must be exclude, include or only"
            ))
        }),
    }
}

/// Check a query's syntax and return its canonical text and, when the builder can show it, its
/// model. A query the builder cannot show has a null `model` and says why in `builder_error`.
pub async fn parse_query(
    body: Result<Json<ParseBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let b = json_body(body)?;
    let q = parse(&b.query)?;
    let (model, builder_error) = match query::Model::from_query(&q) {
        Ok(m) => (json!(m), Value::Null),
        Err(e) => (Value::Null, Value::String(e.to_string())),
    };
    Ok(Json(json!({
        "text": q.to_string(),
        "model": model,
        "builder_error": builder_error,
    })))
}

/// The query text of a builder model.
pub async fn model_text(
    body: Result<Json<TextBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let b = json_body(body)?;
    let text = b
        .model
        .to_text()
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_model", e.to_string()))?;
    Ok(Json(json!({ "text": text })))
}
