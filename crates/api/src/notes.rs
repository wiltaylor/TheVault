//! Notes, their history and the trash (D12, D13, D21, D32).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use state::Author;
use writer::{Event, Input, Saved, Writer, doc_to_json, note_path};

use crate::{ApiError, Caller};

type Shared = State<Arc<Writer>>;

/// Run blocking writer work off the async runtime.
async fn blocking<T: Send + 'static>(
    writer: Arc<Writer>,
    f: impl FnOnce(&Writer) -> writer::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || f(&writer))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(ApiError::from)
}

#[derive(Deserialize)]
pub struct TypeQuery {
    #[serde(rename = "type")]
    type_name: Option<String>,
}

#[derive(Deserialize)]
pub struct BaseQuery {
    base_version: Option<String>,
}

/// A submitted note and what came with it.
struct Submitted {
    input: Input,
    base: Option<String>,
    type_name: Option<String>,
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim_start().starts_with("application/json"))
}

/// The version in `If-Match`, without quotes or a weak prefix.
fn if_match(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::IF_MATCH)?.to_str().ok()?.trim();
    let raw = raw.strip_prefix("W/").unwrap_or(raw);
    Some(raw.trim_matches('"').to_owned()).filter(|s| !s.is_empty())
}

fn submitted(headers: &HeaderMap, body: &Bytes) -> Result<Submitted, ApiError> {
    if !is_json(headers) {
        let text = std::str::from_utf8(body)
            .map_err(|_| ApiError::bad_request("the WCL body is not UTF-8"))?;
        return Ok(Submitted {
            input: Input::Wcl(text.to_owned()),
            base: if_match(headers),
            type_name: None,
        });
    }
    let Value::Object(mut obj) = serde_json::from_slice::<Value>(body)
        .map_err(|e| ApiError::bad_request(format!("the body is not JSON: {e}")))?
    else {
        return Err(ApiError::bad_request("the body must be a JSON object"));
    };
    let string = |obj: &mut serde_json::Map<String, Value>, key: &str| match obj.remove(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ApiError::bad_request(format!("`{key}` must be a string"))),
    };
    let type_name = string(&mut obj, "type")?;
    let base = string(&mut obj, "base_version")?.or_else(|| if_match(headers));
    let input = match (string(&mut obj, "wcl")?, obj.remove("note")) {
        (Some(text), None) => Input::Wcl(text),
        (None, Some(note)) => Input::Json(note),
        _ => {
            return Err(ApiError::bad_request(
                "send exactly one of `note` (JSON) and `wcl` (text)",
            ));
        }
    };
    if let Some(key) = obj.keys().next() {
        return Err(ApiError::bad_request(format!("unexpected key {key:?}")));
    }
    Ok(Submitted {
        input,
        base,
        type_name,
    })
}

/// Whether the client asked for the note as WCL text rather than JSON.
fn wants_wcl(headers: &HeaderMap) -> bool {
    let Some(accept) = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    !accept.contains("application/json")
        && (accept.contains("text/plain") || accept.contains("wcl"))
}

fn event_name(e: Event) -> &'static str {
    match e {
        Event::Create => "create",
        Event::Update => "update",
        Event::Delete => "delete",
    }
}

fn author_json(a: &Author) -> Value {
    let (kind, name) = match a {
        Author::Session(n) => ("session", n),
        Author::Token(n) => ("token", n),
        Author::Handler(n) => ("handler", n),
    };
    json!({ "kind": kind, "name": name })
}

/// A note in the form the client asked for. `extra` adds members to the JSON form.
fn note_response(
    status: StatusCode,
    headers: &HeaderMap,
    id: &str,
    type_name: &str,
    version: &str,
    wcl: String,
    extra: Value,
) -> Response {
    if wants_wcl(headers) {
        let mut res = (status, wcl).into_response();
        let h = res.headers_mut();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        for (name, value) in [
            (header::ETAG.as_str(), format!("\"{version}\"")),
            ("x-note-id", id.to_owned()),
            ("x-note-type", type_name.to_owned()),
        ] {
            if let Ok(v) = HeaderValue::from_str(&value) {
                h.insert(name, v);
            }
        }
        return res;
    }
    let note = Input::Wcl(wcl.clone())
        .parse(&note_path(type_name, id))
        .map(|d| doc_to_json(&d))
        .unwrap_or(Value::Null);
    let mut body = json!({
        "id": id,
        "type": type_name,
        "version": version,
        "wcl": wcl,
        "note": note,
    });
    if let (Value::Object(body), Value::Object(extra)) = (&mut body, extra) {
        body.extend(extra);
    }
    let mut res = (status, Json(body)).into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("\"{version}\"")) {
        res.headers_mut().insert(header::ETAG, v);
    }
    res
}

fn saved_response(status: StatusCode, headers: &HeaderMap, s: Saved) -> Response {
    let extra = json!({ "event": event_name(s.event) });
    note_response(
        status,
        headers,
        &s.id,
        &s.type_name,
        &s.version,
        s.wcl,
        extra,
    )
}

pub async fn list(State(w): Shared, Query(q): Query<TypeQuery>) -> Result<Json<Value>, ApiError> {
    let notes = blocking(w, move |w| {
        if let Some(t) = &q.type_name
            && w.schema().type_def(t).is_none()
        {
            return Err(writer::Error::UnknownType(t.clone()));
        }
        w.list(q.type_name.as_deref())
    })
    .await?;
    let notes: Vec<Value> = notes
        .into_iter()
        .map(|n| {
            json!({
                "id": n.id,
                "type": n.type_name,
                "title": n.title,
                "created": n.created,
                "updated": n.updated,
                "flagged": n.flagged,
                "version": n.version,
            })
        })
        .collect();
    Ok(Json(json!({ "notes": notes })))
}

pub async fn create(
    State(w): Shared,
    Caller(author): Caller,
    Query(q): Query<TypeQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let sub = submitted(&headers, &body)?;
    let type_name = match (q.type_name, sub.type_name) {
        (Some(a), Some(b)) if a != b => {
            return Err(ApiError::bad_request(format!(
                "the type is {a:?} in the URL but {b:?} in the body"
            )));
        }
        (Some(t), _) | (None, Some(t)) => t,
        (None, None) => return Err(ApiError::bad_request("name the note's type with ?type=")),
    };
    let saved = blocking(w, move |w| w.create(&type_name, &sub.input, &author)).await?;
    Ok(saved_response(StatusCode::CREATED, &headers, saved))
}

pub async fn read(
    State(w): Shared,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let note = blocking(w, move |w| w.read(&id)).await?;
    Ok(note_response(
        StatusCode::OK,
        &headers,
        &note.id,
        &note.type_name,
        &note.version,
        note.wcl,
        json!({}),
    ))
}

pub async fn update(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let sub = submitted(&headers, &body)?;
    let saved = blocking(w, move |w| {
        w.update(&id, &sub.input, sub.base.as_deref(), &author)
    })
    .await?;
    Ok(saved_response(StatusCode::OK, &headers, saved))
}

pub async fn delete(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    Query(q): Query<BaseQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let base = q.base_version.or_else(|| if_match(&headers));
    let saved = blocking(w, move |w| w.delete(&id, base.as_deref(), &author)).await?;
    Ok(saved_response(StatusCode::OK, &headers, saved))
}

pub async fn history(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let id2 = id.clone();
    let (versions, current) = blocking(w, move |w| {
        let versions = w.history(&id2)?;
        Ok((versions, w.current_version(&id2)?))
    })
    .await?;
    let versions: Vec<Value> = versions
        .iter()
        .map(|v| {
            json!({
                "version": v.token,
                "type": v.note_type,
                "author": author_json(&v.author),
                "at": v.at_ms,
                "current": current.as_deref() == Some(v.token.as_str()),
            })
        })
        .collect();
    Ok(Json(json!({ "id": id, "versions": versions })))
}

pub async fn version(
    State(w): Shared,
    Path((id, version)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let v = blocking(w, move |w| w.version(&id, &version)).await?;
    let extra = json!({ "author": author_json(&v.author), "at": v.at_ms });
    Ok(note_response(
        StatusCode::OK,
        &headers,
        &v.note_id,
        &v.note_type,
        &v.token,
        v.content,
        extra,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreBody {
    version: String,
    base_version: Option<String>,
}

pub async fn restore_version(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RestoreBody>,
) -> Result<Response, ApiError> {
    let base = body.base_version.or_else(|| if_match(&headers));
    let saved = blocking(w, move |w| {
        w.restore_version(&id, &body.version, base.as_deref(), &author)
    })
    .await?;
    Ok(saved_response(StatusCode::OK, &headers, saved))
}

pub async fn trash(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let entries = blocking(w, |w| w.trash()).await?;
    let notes: Vec<Value> = entries
        .into_iter()
        .map(|t| {
            json!({
                "id": t.note_id,
                "type": t.note_type,
                "version": t.version,
                "wcl": t.content,
                "deleted_by": author_json(&t.deleted_by),
                "deleted_at": t.deleted_ms,
            })
        })
        .collect();
    Ok(Json(json!({ "notes": notes })))
}

pub async fn restore_from_trash(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let saved = blocking(w, move |w| w.restore_from_trash(&id, &author)).await?;
    Ok(saved_response(StatusCode::OK, &headers, saved))
}
