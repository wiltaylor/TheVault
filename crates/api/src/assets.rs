//! Uploaded files (D22): `assets/<ULID>/<filename>`, stored and served byte for byte.
//!
//! `POST /api/assets?filename=<name>` takes the file's bytes as the body. A download is served
//! with a sandboxing content security policy, so an uploaded HTML or SVG file cannot run script
//! with the owner's session.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{Asset, Writer};

use crate::error::in_use;
use crate::{ApiError, Caller, blocking};

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

/// `PUT /api/assets/{id}?filename=<name>`: store the body as asset `id`, an id the client chose,
/// so a device keeps the id of an asset it stored offline when it syncs (D19, D20). Sending the
/// same file again answers 200; anything else already under the id is 409 `id_taken`.
pub async fn put(
    State(w): Shared,
    Path(id): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let filename = q
        .filename
        .ok_or_else(|| ApiError::bad_request("name the file with ?filename="))?;
    let id2 = id.clone();
    let (existed, asset) = blocking(w, move |w| {
        let existed = w.asset_info(&id2).is_ok();
        Ok::<_, writer::Error>((
            existed,
            w.put_asset_with_id(&id2, &filename, body.to_vec())?,
        ))
    })
    .await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(asset_json(&asset))).into_response())
}

/// `GET /api/assets`.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let assets = blocking(w, |w| w.assets()).await?;
    Ok(Json(
        json!({ "assets": assets.iter().map(asset_json).collect::<Vec<_>>() }),
    ))
}

/// `GET /api/assets/{id}`: the file's bytes. A `Range: bytes=…` header asking for one range
/// gets just that range, as `206 Partial Content`, which lets pdf.js fetch a large PDF a piece at
/// a time. Several ranges, or a header that does not parse, get the whole file.
pub async fn download(
    State(w): Shared,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let (asset, part, bytes) = blocking(w, move |w| {
        let asset = w.asset_info(&id)?;
        let part = range
            .as_deref()
            .map_or(Part::Whole, |r| parse_range(r, asset.size));
        let bytes = match part {
            Part::Whole => w.asset(&id)?.1,
            Part::Range(start, end) => w.asset_range(&id, start, end - start + 1)?.1,
            Part::Unsatisfiable => Vec::new(),
        };
        Ok::<_, writer::Error>((asset, part, bytes))
    })
    .await?;
    let size = asset.size;
    let mut res = match part {
        Part::Whole => bytes.into_response(),
        Part::Range(start, end) => {
            let mut res = (StatusCode::PARTIAL_CONTENT, bytes).into_response();
            if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}")) {
                res.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            res
        }
        Part::Unsatisfiable => {
            let mut res = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}")) {
                res.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            return Ok(res);
        }
    };
    let h = res.headers_mut();
    let ty = content_type(&asset.filename);
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ty));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(v) = HeaderValue::from_str(&format!(
        "inline; filename*=UTF-8''{}",
        percent_encode(&asset.filename)
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    // The browser's own PDF viewer refuses to run in a sandboxed document, and it never runs a
    // PDF's script with the page's origin, so a PDF is served without the sandbox.
    if ty != "application/pdf" {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "sandbox; default-src 'none'; img-src 'self'; style-src 'unsafe-inline'",
            ),
        );
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(res)
}

/// What a `Range` header asks of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The whole file: no header, several ranges, or one that does not parse.
    Whole,
    /// Bytes `start..=end`, inside the file.
    Range(u64, u64),
    /// A range that starts past the end of the file.
    Unsatisfiable,
}

/// Read a `Range` header for a file of `size` bytes (RFC 9110, section 14.1.2). Only one range
/// of bytes is served on its own.
fn parse_range(header: &str, size: u64) -> Part {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return Part::Whole;
    };
    if spec.contains(',') {
        return Part::Whole;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return Part::Whole;
    };
    let num = |s: &str| {
        let s = s.trim();
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u64>().ok())
            .flatten()
    };
    match (first.trim().is_empty(), num(first), num(last)) {
        // `-n`: the last n bytes.
        (true, _, Some(n)) => {
            if n == 0 || size == 0 {
                Part::Unsatisfiable
            } else {
                Part::Range(size.saturating_sub(n), size - 1)
            }
        }
        (false, Some(start), end) if last.trim().is_empty() || end.is_some() => {
            if start >= size {
                return Part::Unsatisfiable;
            }
            match end {
                Some(end) if end < start => Part::Whole,
                Some(end) => Part::Range(start, end.min(size - 1)),
                None => Part::Range(start, size - 1),
            }
        }
        _ => Part::Whole,
    }
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    unlink: bool,
}

/// `DELETE /api/assets/{id}[?unlink=true]`. Notes naming the asset refuse the delete, listing
/// them, unless `unlink` is set: then they lose their references to it in the same commit.
pub async fn delete(
    State(w): Shared,
    Caller(author): Caller,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> Result<Json<Value>, ApiError> {
    let (asset, saved) = blocking(w, move |w| {
        w.delete_asset(&id, q.unlink, &author)
            .map_err(|e| in_use(w, e))
    })
    .await?;
    let mut body = asset_json(&asset);
    body["deleted"] = json!(true);
    body["notes"] = saved
        .iter()
        .map(|s| json!({ "id": s.id, "type": s.type_name, "version": s.version }))
        .collect();
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
    fn reads_one_byte_range() {
        assert_eq!(parse_range("bytes=0-99", 1000), Part::Range(0, 99));
        assert_eq!(parse_range("bytes=900-", 1000), Part::Range(900, 999));
        assert_eq!(parse_range("bytes=900-5000", 1000), Part::Range(900, 999));
        assert_eq!(parse_range("bytes=-100", 1000), Part::Range(900, 999));
        assert_eq!(parse_range("bytes=-5000", 1000), Part::Range(0, 999));
        assert_eq!(parse_range("bytes=1000-", 1000), Part::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 1000), Part::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), Part::Whole);
        assert_eq!(parse_range("bytes=5-1", 1000), Part::Whole);
        assert_eq!(parse_range("items=0-1", 1000), Part::Whole);
        assert_eq!(parse_range("bytes=x-1", 1000), Part::Whole);
        assert_eq!(parse_range("bytes=0-0", 0), Part::Unsatisfiable);
    }

    #[test]
    fn serves_pdf_as_pdf() {
        assert_eq!(content_type("Report.PDF"), "application/pdf");
    }

    #[test]
    fn never_serves_html_as_html() {
        assert_eq!(content_type("x.HTML"), "text/plain; charset=utf-8");
        assert_eq!(content_type("photo.JPG"), "image/jpeg");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
