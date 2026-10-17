//! Templates (D14), taxonomies with their hierarchy edits (D7), scripts, and note canvases (D15).
//!
//! Every write here takes the WCL (or for a script, the wscript source) as a text body, or as
//! JSON `{ "wcl": "..." }` with any extra members named below. Every read answers JSON, or the
//! bare text when `Accept` asks for `text/plain` or WCL.
//!
//! - `PUT /api/types/{name}/templates/{template}`: one `template <template> { ... }` block.
//! - `PUT /api/taxonomies/{name}`: the taxonomy file, with `renames`, a map of old term paths to
//!   new ones, to rename or move terms. Notes holding a renamed term are rewritten in the same
//!   commit, and `notes` in the response lists them.
//! - `PUT /api/scripts/{name}`: the script's source, as `{ "source": "..." }` in JSON. It is stored
//!   as sent. The JSON answer to a read or a write of a script carries `problems`: why it would not
//!   compile, once for each way the vault runs it (`before_handler`, `after_handler`, `task`).
//! - `PUT /api/notes/{id}/canvas/{name}`: one `canvas { ... }` block, with `base_version` or
//!   `If-Match`. `name` is a canvas field, or `<content field>.<n>` for the n-th canvas block
//!   inside a content field.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use writer::Writer;

use crate::error::in_use;
use crate::inspect::{taxonomy_json, template_json};
use crate::notes::{if_match, is_json, wants_wcl};
use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// A JSON body as `T`, or a text body turned into `T` by `from_text`.
pub(crate) fn body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
    from_text: impl FnOnce(String) -> T,
) -> Result<T, ApiError> {
    if is_json(headers) {
        return serde_json::from_slice(body)
            .map_err(|e| ApiError::bad_request(format!("the JSON body does not fit: {e}")));
    }
    let text = String::from_utf8(body.to_vec())
        .map_err(|_| ApiError::bad_request("the text body is not UTF-8"))?;
    Ok(from_text(text))
}

/// `json`, or its `text` member alone when the client asked for text.
pub(crate) fn text_or_json(headers: &HeaderMap, text: String, json: Value) -> Response {
    if wants_wcl(headers) {
        let mut res = text.into_response();
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        return res;
    }
    Json(json).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WclBody {
    pub(crate) wcl: String,
}

// ---- templates ------------------------------------------------------------------------------

/// `GET /api/types/{name}/templates`: every template of the type, laid out.
pub async fn templates(
    State(w): Shared,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    let ty = blocking(w, move |w| {
        w.schema()
            .type_def(&n)
            .cloned()
            .ok_or_else(|| writer::Error::Missing(format!("there is no type {n:?}")))
    })
    .await?;
    let templates: Vec<Value> = ty.templates.iter().map(template_json).collect();
    Ok(Json(json!({ "type": name, "templates": templates })))
}

/// `GET /api/types/{name}/templates/{template}`.
pub async fn template(
    State(w): Shared,
    Path((name, template)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (n, t) = (name.clone(), template.clone());
    let wcl = blocking(w, move |w| w.template(&n, &t)).await?;
    let json = json!({ "type": name, "name": template, "wcl": wcl });
    Ok(text_or_json(&headers, wcl, json))
}

/// `PUT /api/types/{name}/templates/{template}`.
pub async fn put_template(
    State(w): Shared,
    Path((name, template)): Path<(String, String)>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: WclBody = body(&headers, &raw, |wcl| WclBody { wcl })?;
    let (n, t) = (name.clone(), template.clone());
    let wcl = blocking(w, move |w| w.put_template(&n, &t, &b.wcl)).await?;
    let json = json!({ "type": name, "name": template, "wcl": wcl });
    Ok(text_or_json(&headers, wcl, json))
}

/// `DELETE /api/types/{name}/templates/{template}`.
pub async fn delete_template(
    State(w): Shared,
    Path((name, template)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let (n, t) = (name.clone(), template.clone());
    blocking(w, move |w| w.delete_template(&n, &t)).await?;
    Ok(Json(
        json!({ "type": name, "name": template, "deleted": true }),
    ))
}

// ---- taxonomies -----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaxonomyBody {
    wcl: String,
    #[serde(default)]
    renames: BTreeMap<String, String>,
}

/// `GET /api/taxonomies`: every taxonomy with its term tree.
pub async fn taxonomies(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let schema = blocking(w, |w| Ok::<_, writer::Error>(w.schema())).await?;
    let list: Vec<Value> = schema.taxonomies().map(taxonomy_json).collect();
    Ok(Json(json!({ "taxonomies": list })))
}

/// `GET /api/taxonomies/{name}`.
pub async fn taxonomy(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let n = name.clone();
    let (wcl, schema) = blocking(w, move |w| {
        Ok::<_, writer::Error>((w.read_taxonomy(&n)?, w.schema()))
    })
    .await?;
    let mut json = schema
        .taxonomy(&name)
        .map(taxonomy_json)
        .unwrap_or_else(|| json!({}));
    json["wcl"] = json!(wcl);
    Ok(text_or_json(&headers, wcl, json))
}

/// `PUT /api/taxonomies/{name}`: create or replace a taxonomy, renaming terms in notes.
pub async fn put_taxonomy(
    State(w): Shared,
    Caller(author): Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: TaxonomyBody = body(&headers, &raw, |wcl| TaxonomyBody {
        wcl,
        renames: BTreeMap::new(),
    })?;
    let n = name.clone();
    let (wcl, saved, schema) = blocking(w, move |w| {
        let (wcl, saved) = w
            .put_taxonomy(&n, &b.wcl, &b.renames, &author)
            .map_err(|e| in_use(w, e))?;
        Ok::<_, ApiError>((wcl, saved, w.schema()))
    })
    .await?;
    let mut json = schema
        .taxonomy(&name)
        .map(taxonomy_json)
        .unwrap_or_else(|| json!({}));
    json["wcl"] = json!(wcl);
    json["notes"] = saved
        .iter()
        .map(|s| json!({ "id": s.id, "type": s.type_name, "version": s.version }))
        .collect();
    Ok(text_or_json(&headers, wcl, json))
}

/// `DELETE /api/taxonomies/{name}`.
pub async fn delete_taxonomy(
    State(w): Shared,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    blocking(w, move |w| w.delete_taxonomy(&n).map_err(|e| in_use(w, e))).await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

// ---- scripts --------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptBody {
    source: String,
}

/// `GET /api/scripts`.
pub async fn scripts(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let names = blocking(w, |w| w.scripts()).await?;
    Ok(Json(json!({ "scripts": names })))
}

/// `GET /api/scripts/{name}`.
pub async fn script(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let n = name.clone();
    let (source, problems) = blocking(w, move |w| {
        let source = w.read_script(&n)?;
        let problems = w.script_problems(&n, &source)?;
        Ok::<_, writer::Error>((source, problems))
    })
    .await?;
    let json = json!({ "name": name, "source": source, "problems": problems_json(&problems) });
    Ok(text_or_json(&headers, source, json))
}

/// Why a script would not compile, once for each way the vault runs it.
pub(crate) fn problems_json(problems: &[writer::ScriptProblem]) -> Value {
    problems
        .iter()
        .map(|p| json!({ "role": p.role.as_str(), "message": p.message }))
        .collect()
}

/// `PUT /api/scripts/{name}`.
pub async fn put_script(
    State(w): Shared,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: ScriptBody = body(&headers, &raw, |source| ScriptBody { source })?;
    let n = name.clone();
    let (source, problems) = blocking(w, move |w| {
        let source = w.put_script(&n, &b.source)?;
        let problems = w.script_problems(&n, &source)?;
        Ok::<_, writer::Error>((source, problems))
    })
    .await?;
    let json = json!({ "name": name, "source": source, "problems": problems_json(&problems) });
    Ok(text_or_json(&headers, source, json))
}

/// `DELETE /api/scripts/{name}`.
pub async fn delete_script(
    State(w): Shared,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    blocking(w, move |w| w.delete_script(&n)).await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

// ---- canvases -------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasBody {
    wcl: String,
    base_version: Option<String>,
}

fn canvas_response(
    status: StatusCode,
    headers: &HeaderMap,
    id: &str,
    name: &str,
    version: &str,
    wcl: String,
) -> Response {
    let canvas = writer::Input::Wcl(wcl.clone())
        .parse("canvas.wcl")
        .ok()
        .and_then(|d| d.blocks.first().map(writer::block_to_json))
        .unwrap_or(Value::Null);
    let json = json!({ "id": id, "name": name, "version": version, "wcl": wcl, "canvas": canvas });
    let mut res = text_or_json(headers, wcl, json);
    *res.status_mut() = status;
    if let Ok(v) = HeaderValue::from_str(&format!("\"{version}\"")) {
        res.headers_mut().insert(header::ETAG, v);
    }
    res
}

/// `GET /api/notes/{id}/canvas/{name}`.
pub async fn canvas(
    State(w): Shared,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (i, n) = (id.clone(), name.clone());
    let (version, wcl) = blocking(w, move |w| w.canvas(&i, &n)).await?;
    Ok(canvas_response(
        StatusCode::OK,
        &headers,
        &id,
        &name,
        &version,
        wcl,
    ))
}

/// `PUT /api/notes/{id}/canvas/{name}`: replace one canvas of a note.
pub async fn put_canvas(
    State(w): Shared,
    Caller(author): Caller,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: CanvasBody = body(&headers, &raw, |wcl| CanvasBody {
        wcl,
        base_version: None,
    })?;
    let base = b.base_version.or_else(|| if_match(&headers));
    let (i, n) = (id.clone(), name.clone());
    let (saved, wcl) = blocking(w, move |w| {
        let saved = w.put_canvas(&i, &n, &b.wcl, base.as_deref(), &author)?;
        let (_, wcl) = w.canvas(&i, &n)?;
        Ok::<_, writer::Error>((saved, wcl))
    })
    .await?;
    Ok(canvas_response(
        StatusCode::OK,
        &headers,
        &id,
        &name,
        &saved.version,
        wcl,
    ))
}
