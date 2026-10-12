//! Uploaded files (D22): `assets/<ULID>/<filename>`, stored and served byte for byte.
//!
//! `POST /api/assets?filename=<name>` takes the file's bytes as the body. A download is served
//! with a sandboxing content security policy, so an uploaded HTML or SVG file cannot run script
//! with the owner's session.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{Asset, Writer};

use crate::{ApiError, blocking};

type Shared = State<Arc<Writer>>;

/// The largest upload accepted, in bytes.
pub const MAX_UPLOAD: usize = 512 * 1024 * 1024;

#[derive(Deserialize)]
pub struct UploadQuery {
    filename: Option<String>,
}

fn asset_json(a: &Asset) -> Value {
    json!({ "id": a.id, "filename": a.filename, "size": a.size, "path": a.path() })
}

/// `POST /api/assets?filename=<name>`.
pub async fn upload(
    State(w): Shared,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let filename = q
        .filename
        .ok_or_else(|| ApiError::bad_request("name the file with ?filename="))?;
    let asset = blocking(w, move |w| w.put_asset(&filename, body.to_vec())).await?;
    Ok((StatusCode::CREATED, Json(asset_json(&asset))).into_response())
}

/// `GET /api/assets`.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let assets = blocking(w, |w| w.assets()).await?;
    Ok(Json(
        json!({ "assets": assets.iter().map(asset_json).collect::<Vec<_>>() }),
    ))
}

/// `GET /api/assets/{id}`: the file's bytes.
pub async fn download(State(w): Shared, Path(id): Path<String>) -> Result<Response, ApiError> {
    let (asset, bytes) = blocking(w, move |w| w.asset(&id)).await?;
    let mut res = bytes.into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(&asset.filename)),
    );
    if let Ok(v) = HeaderValue::from_str(&format!(
        "inline; filename*=UTF-8''{}",
        percent_encode(&asset.filename)
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox; default-src 'none'; img-src 'self'; style-src 'unsafe-inline'",
        ),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(res)
}

/// `DELETE /api/assets/{id}`.
pub async fn delete(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let asset = blocking(w, move |w| w.delete_asset(&id)).await?;
    let mut body = asset_json(&asset);
    body["deleted"] = json!(true);
    Ok(Json(body))
}

/// The media type served for a file name. HTML is served as plain text.
fn content_type(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" | "md" | "csv" | "html" | "htm" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// RFC 8187 encoding for `filename*`: unreserved ASCII as is, every other byte as `%XX`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_file_names_for_the_disposition_header() {
        assert_eq!(percent_encode("a b.png"), "a%20b.png");
        assert_eq!(percent_encode("ü\"x"), "%C3%BC%22x");
    }

    #[test]
    fn never_serves_html_as_html() {
        assert_eq!(content_type("x.HTML"), "text/plain; charset=utf-8");
        assert_eq!(content_type("photo.JPG"), "image/jpeg");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
