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

use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

#[derive(Deserialize)]
pub struct TypeQuery {
    #[serde(rename = "type")]
    type_name: Option<String>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(rename = "type")]
    type_name: Option<String>,
    /// `exclude` (the default), `include` or `only`.
    archived: Option<String>,
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

pub(crate) fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim_start().starts_with("application/json"))
}

/// The version in `If-Match`, without quotes or a weak prefix.
pub(crate) fn if_match(headers: &HeaderMap) -> Option<String> {
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
pub(crate) fn wants_wcl(headers: &HeaderMap) -> bool {
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
    let (kind, name) = a.parts();
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

/// A saved note. `also` lists the other notes the write changed, without their WCL.
fn saved_response(status: StatusCode, headers: &HeaderMap, s: Saved) -> Response {
    let also: Vec<Value> = s
        .also
        .iter()
        .map(|o| {
            json!({
                "id": o.id,
                "type": o.type_name,
                "event": event_name(o.event),
                "version": o.version,
            })
        })
        .collect();
    let extra = json!({ "event": event_name(s.event), "also": also });
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

pub async fn list(State(w): Shared, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let archived = crate::query::archived_param(q.archived.as_deref())?;
    let notes = blocking(w, move |w| {
        if let Some(t) = &q.type_name
            && w.schema().type_def(t).is_none()
        {
            return Err(writer::Error::UnknownType(t.clone()));
        }
        w.list_with(q.type_name.as_deref(), archived)
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
                "archived": n.archived,
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

/// Whether the request carries `If-None-Match: *`: write only if the note does not exist.
fn if_none_match_any(headers: &HeaderMap) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim() == "*")
}

/// `PUT /api/notes/{id}`: replace the note at the base version in `If-Match` or `base_version`.
/// With `If-None-Match: *` and the type in `?type=` or the body, create the note under `id`
/// instead, which must be a ULID no note has ever had (409 `id_taken` otherwise): a device keeps
/// the id of a note it created offline when it syncs (D20).
pub async fn update(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    Query(q): Query<TypeQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let sub = submitted(&headers, &body)?;
    if if_none_match_any(&headers) {
        let type_name = match (q.type_name, sub.type_name) {
            (Some(a), Some(b)) if a != b => {
                return Err(ApiError::bad_request(format!(
                    "the type is {a:?} in the URL but {b:?} in the body"
                )));
            }
            (Some(t), _) | (None, Some(t)) => t,
            (None, None) => {
                return Err(ApiError::bad_request("name the note's type with ?type="));
            }
        };
        let saved = blocking(w, move |w| {
            w.create_with_id(&id, &type_name, &sub.input, &author)
        })
        .await?;
        return Ok(saved_response(StatusCode::CREATED, &headers, saved));
    }
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

/// `POST /api/notes/{id}/archive`: archive a note at its base version, as a new version.
pub async fn archive(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    Query(q): Query<BaseQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    set_archived(w, author, id, q, headers, true).await
}

/// `POST /api/notes/{id}/unarchive`: take a note out of the archive at its base version.
pub async fn unarchive(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    Query(q): Query<BaseQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    set_archived(w, author, id, q, headers, false).await
}

async fn set_archived(
    w: Arc<Writer>,
    author: Author,
    id: String,
    q: BaseQuery,
    headers: HeaderMap,
    archived: bool,
) -> Result<Response, ApiError> {
    let base = q.base_version.or_else(|| if_match(&headers));
    let saved = blocking(w, move |w| {
        w.set_archived(&id, archived, base.as_deref(), &author)
    })
    .await?;
    Ok(saved_response(StatusCode::OK, &headers, saved))
}

pub async fn history(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let id2 = id.clone();
    let (versions, current) = blocking(w, move |w| -> writer::Result<_> {
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

/// The most neighbours a graph holds when the request does not say.
const GRAPH_LIMIT: usize = 60;
/// The most neighbours a graph holds whatever the request says.
const GRAPH_MAX: usize = 500;

#[derive(Deserialize)]
pub struct GraphQuery {
    limit: Option<usize>,
}

fn graph_node(n: &writer::GraphNode, direction: &str) -> Value {
    json!({
        "id": n.id,
        "type": n.type_name,
        "title": n.title,
        "updated": n.updated,
        "direction": direction,
    })
}

/// `GET /api/notes/{id}/graph`: the note, the notes it links to or that link to it, and every link
/// among them. Each neighbour's `direction` says how it links to the note: `out` (the note links
/// to it), `in` (it links to the note) or `both`. Placeholder links are not links, and notes in
/// the trash are left out.
pub async fn graph(
    State(w): Shared,
    Path(id): Path<String>,
    Query(q): Query<GraphQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = q.limit.unwrap_or(GRAPH_LIMIT).min(GRAPH_MAX);
    let g = blocking(w, move |w| w.graph(&id, limit)).await?;
    let c = &g.center.id;
    let direction = |n: &str| {
        let out = g.edges.iter().any(|e| &e.from == c && e.to == n);
        let inc = g.edges.iter().any(|e| e.from == n && &e.to == c);
        match (out, inc) {
            (true, true) => "both",
            (true, false) => "out",
            _ => "in",
        }
    };
    let mut nodes = vec![graph_node(&g.center, "center")];
    nodes.extend(g.neighbours.iter().map(|n| graph_node(n, direction(&n.id))));
    let edges: Vec<Value> = g
        .edges
        .iter()
        .map(|e| json!({ "from": e.from, "to": e.to, "via": e.via, "field": e.field }))
        .collect();
    Ok(Json(json!({
        "id": g.center.id,
        "nodes": nodes,
        "edges": edges,
        "truncated": g.truncated,
    })))
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

/// Refuse a caller that is not a password-login session. [`crate::auth::authenticate`] refuses
/// them first; this keeps the rule should the routes be reached another way (ai D21).
fn human_only(author: &Author) -> Result<(), ApiError> {
    match author {
        Author::Session(_) => Ok(()),
        _ => Err(crate::auth::human_only()),
    }
}

fn purged_json(p: writer::Purged) -> Json<Value> {
    let notes: Vec<&str> = p.notes.iter().map(|t| t.note_id.as_str()).collect();
    let assets: Vec<&str> = p.assets.iter().map(|a| a.id.as_str()).collect();
    Json(json!({ "purged": notes, "assets_trashed": assets }))
}

/// `POST /api/trash/empty`: purge every note in the trash, with its history, for good.
pub async fn empty_trash(
    State(w): Shared,
    Caller(author): Caller,
) -> Result<Json<Value>, ApiError> {
    human_only(&author)?;
    Ok(purged_json(blocking(w, |w| w.empty_trash()).await?))
}

/// `DELETE /api/trash/{id}`: purge one note in the trash, with its history, for good.
pub async fn purge_from_trash(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    human_only(&author)?;
    Ok(purged_json(
        blocking(w, move |w| w.purge_from_trash(&id)).await?,
    ))
}

/// `GET /api/daily/{date}`: the daily note for a day, given as `YYYY-MM-DD`. A day with no note is
/// a 404; `POST` creates it.
pub async fn daily(
    State(w): Shared,
    Path(date): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let day = date.clone();
    let Some(note) = blocking(w, move |w| w.find_daily_note(&day)).await? else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("there is no daily note for {date}"),
        ));
    };
    Ok(note_response(
        StatusCode::OK,
        &headers,
        &note.id,
        &note.type_name,
        &note.version,
        note.wcl,
        json!({ "created": false }),
    ))
}

/// `POST /api/daily/{date}`: the daily note for a day, created when the day has none. Calling it
/// again for the same day returns the same note.
pub async fn open_daily(
    State(w): Shared,
    Caller(author): Caller,
    Path(date): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (note, created) = blocking(w, move |w| w.daily_note(&date, &author)).await?;
    Ok(note_response(
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        &headers,
        &note.id,
        &note.type_name,
        &note.version,
        note.wcl,
        json!({ "created": created }),
    ))
}
