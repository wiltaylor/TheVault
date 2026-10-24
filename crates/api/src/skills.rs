//! Agent Skills, `skills/<name>/` (skills S1, S2, S3).
//!
//! - `GET /api/skills` (read): `{ "skills": [summary] }`, every skill sorted by name, valid or not.
//!   A summary is `{ name, description, license, compatibility, metadata, allowed_tools, extra,
//!   files, file_count, size, valid, issues }`: the frontmatter fields (`null` when unset or not
//!   a string; `metadata` the string values; `extra` every other frontmatter key with its value
//!   as YAML text), the number of files (as both `files` and `file_count`) and their total bytes,
//!   whether the skill has no errors, and each issue as `{ at, message, severity }`.
//! - `GET /api/skills/{name}` (read): the summary plus `body`, the Markdown after the
//!   frontmatter, and `files` as a list in place of the count, each `{ path, size, media_type,
//!   text }`, `text` saying whether it reads as UTF-8 text.
//! - `GET /api/skills/{name}/files/{*path}` (read): the file's bytes, served sandboxed.
//! - `GET /api/skills/{name}/export` (read): the skill as a zip of one folder, `<name>/`.
//! - `PUT /api/skills/{name}` (admin): the whole `SKILL.md` text as the body (or JSON
//!   `{ "text": "..." }`), validated: an error is a 400 `invalid` with its issues, and nothing is
//!   written. Creates the skill when it does not exist (201) or replaces `SKILL.md` (200);
//!   answers the skill as `GET` does.
//! - `PUT /api/skills/{name}/files/{*path}` (admin): the body as the file's bytes. The skill must
//!   exist. Answers `{ name, file }`.
//! - `DELETE /api/skills/{name}/files/{*path}` (admin): the file, or every file under the folder
//!   `path`. `SKILL.md` is refused. Answers `{ name, deleted: [paths] }`.
//! - `DELETE /api/skills/{name}` (admin): the whole folder. Answers `{ name, deleted: true }`.
//! - `POST /api/skills/import?replace=true` (admin): a zip, as the body or as the first file of a
//!   `multipart/form-data` body (whose `replace` field also counts). It holds one top-level folder
//!   with `SKILL.md`, or `SKILL.md` at its root, and is named by the frontmatter `name`. An
//!   existing name is a 409 `skill_exists`, with the skill's `name`, unless `replace`. Everything is checked before
//!   anything is written. Answers the skill as `GET` does with `replaced`: 201 for a new skill,
//!   200 for a replaced one.
//!
//! A limit broken is a 413 `too_large`; a bad name or path a 400 `bad_request`; a missing skill
//! or file a 404 `not_found`. Writes need `admin`, as agent files do (agents G2), so the apps show
//! skills read-only (`auth::required_scope` answers `admin` for any write it does not list).

use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, Multipart, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{Skill, SkillFileInfo, Writer};

use crate::assets::percent_encode;
use crate::error::issue_json;
use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// The largest file `PUT /api/skills/{name}/files/{*path}` takes: a whole skill's worth.
pub const MAX_FILE_UPLOAD: usize = store::MAX_SKILL_BYTES as usize;

/// The largest `SKILL.md` `PUT /api/skills/{name}` takes.
pub const MAX_SKILL_MD: usize = store::MAX_SKILL_TEXT_BYTES as usize;

/// The largest import body: the zip, with room for a multipart envelope.
pub const MAX_IMPORT: usize = writer::MAX_SKILL_ZIP + 64 * 1024;

/// A skill's line in `GET /api/skills`, and the head of every other answer about it.
pub fn summary_json(s: &Skill) -> Value {
    json!({
        "name": s.name,
        "description": s.md.description,
        "license": s.md.license,
        "compatibility": s.md.compatibility,
        "metadata": s.md.metadata,
        "allowed_tools": s.md.allowed_tools,
        "extra": s.md.extra,
        "files": s.files.len(),
        "file_count": s.files.len(),
        "size": s.size,
        "valid": s.valid(),
        "issues": s.issues.iter().map(issue_json).collect::<Vec<_>>(),
    })
}

fn file_json(f: &SkillFileInfo) -> Value {
    json!({ "path": f.path, "size": f.size, "media_type": f.media_type, "text": f.text })
}

/// A skill as `GET /api/skills/{name}` answers it.
pub fn skill_json(s: &Skill) -> Value {
    let mut out = summary_json(s);
    out["body"] = json!(s.md.body);
    out["files"] = s.files.iter().map(file_json).collect();
    out
}

/// `GET /api/skills`.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let skills = blocking(w, |w| w.skills()).await?;
    let skills: Vec<Value> = skills.iter().map(summary_json).collect();
    Ok(Json(json!({ "skills": skills })))
}

/// `GET /api/skills/{name}`.
pub async fn read(State(w): Shared, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    let skill = blocking(w, move |w| w.skill(&name)).await?;
    Ok(Json(skill_json(&skill)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillMdBody {
    text: String,
}

/// `PUT /api/skills/{name}`.
pub async fn put(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, ApiError> {
    let b: SkillMdBody = crate::defs::body(&headers, &raw, |text| SkillMdBody { text })?;
    let (skill, created) = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.put_skill_md(&name, &b.text)
    })
    .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(skill_json(&skill))).into_response())
}

/// `DELETE /api/skills/{name}`.
pub async fn delete(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.delete_skill(&n)
    })
    .await?;
    Ok(Json(json!({ "name": name, "deleted": true })))
}

/// `GET /api/skills/{name}/files/{*path}`: served with a sandboxing content security policy, so
/// an HTML or SVG file in a skill cannot run script with the owner's session, and HTML as text.
pub async fn file(
    State(w): Shared,
    Path((name, path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let p = path.clone();
    let bytes = blocking(w, move |w| w.skill_file(&name, &p)).await?;
    let media = match writer::skill_media_type(&path) {
        "text/html" => "text/plain; charset=utf-8",
        m if m.starts_with("text/") => {
            return Ok(served(bytes, &format!("{m}; charset=utf-8"), &path));
        }
        m => m,
    };
    Ok(served(bytes, media, &path))
}

fn served(bytes: Vec<u8>, media: &str, path: &str) -> Response {
    let mut res = Body::from(bytes).into_response();
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(media) {
        h.insert(header::CONTENT_TYPE, v);
    }
    let leaf = path.rsplit('/').next().unwrap_or(path);
    if let Ok(v) = HeaderValue::from_str(&format!(
        "inline; filename*=UTF-8''{}",
        percent_encode(leaf)
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'; img-src 'self'"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

/// `PUT /api/skills/{name}/files/{*path}`.
pub async fn put_file(
    State(w): Shared,
    Caller(by): Caller,
    Path((name, path)): Path<(String, String)>,
    raw: Bytes,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    let file = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.put_skill_file(&n, &path, &raw)
    })
    .await?;
    Ok(Json(json!({ "name": name, "file": file_json(&file) })))
}

/// `DELETE /api/skills/{name}/files/{*path}`.
pub async fn delete_file(
    State(w): Shared,
    Caller(by): Caller,
    Path((name, path)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let n = name.clone();
    let deleted = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.delete_skill_file(&n, &path)
    })
    .await?;
    Ok(Json(json!({ "name": name, "deleted": deleted })))
}

#[derive(Deserialize)]
pub struct ImportQuery {
    #[serde(default)]
    replace: Option<String>,
}

fn truthy(v: &str) -> Result<bool, ApiError> {
    match v {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" | "" => Ok(false),
        other => Err(ApiError::bad_request(format!(
            "replace must be true or false, not {other:?}"
        ))),
    }
}

/// `POST /api/skills/import`.
pub async fn import(
    State(w): Shared,
    Caller(by): Caller,
    Query(q): Query<ImportQuery>,
    req: Request,
) -> Result<Response, ApiError> {
    let mut replace = q
        .replace
        .as_deref()
        .map(truthy)
        .transpose()?
        .unwrap_or(false);
    let multipart = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim_start().starts_with("multipart/form-data"));
    let zip: Vec<u8> = if multipart {
        let bad = |e: axum::extract::multipart::MultipartError| {
            ApiError::bad_request(format!("reading the form: {e}"))
        };
        let mut form = Multipart::from_request(req, &())
            .await
            .map_err(|e| ApiError::bad_request(format!("reading the form: {e}")))?;
        let mut zip = None;
        while let Some(field) = form.next_field().await.map_err(bad)? {
            let is_file = field.file_name().is_some();
            match field.name() {
                Some("replace") if !is_file => {
                    replace = replace || truthy(field.text().await.map_err(bad)?.trim())?;
                }
                _ if zip.is_none() && (is_file || field.name() == Some("file")) => {
                    zip = Some(field.bytes().await.map_err(bad)?.to_vec());
                }
                _ => {}
            }
        }
        zip.ok_or_else(|| ApiError::bad_request("the form holds no zip file"))?
    } else {
        axum::body::to_bytes(req.into_body(), MAX_IMPORT)
            .await
            .map_err(|e| ApiError::bad_request(format!("reading the zip: {e}")))?
            .to_vec()
    };
    if zip.len() > writer::MAX_SKILL_ZIP {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!("a skill zip may be at most {} bytes", writer::MAX_SKILL_ZIP),
        ));
    }
    let (skill, replaced) = blocking(w, move |w| {
        let _acting = writer::acting_as(&by);
        w.import_skill_zip(&zip, replace)
    })
    .await?;
    let mut out = skill_json(&skill);
    out["replaced"] = json!(replaced);
    let status = if replaced {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(out)).into_response())
}

/// `GET /api/skills/{name}/export`.
pub async fn export(State(w): Shared, Path(name): Path<String>) -> Result<Response, ApiError> {
    let n = name.clone();
    let zip = blocking(w, move |w| w.export_skill_zip(&n)).await?;
    let mut res = Body::from(zip).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!(
        "attachment; filename*=UTF-8''{}.zip",
        percent_encode(&name)
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}
