//! Uploaded files (D22): `assets/<ULID>/<filename>`, stored and served byte for byte.
//!
//! `POST /api/assets?filename=<name>` takes the file's bytes as the body. A download is served
//! with a sandboxing content security policy, so an uploaded HTML or SVG file cannot run script
//! with the owner's session.
//!
//! Neither direction holds a whole file in memory (ticket 142): an upload streams into a file in
//! `.vault/incoming` ([`Writer::incoming_file`]), which the commit then moves into place, and a
//! download streams from the asset's file a block at a time.

use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::{Asset, AssetSummary, TrashedAsset, Writer};

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

/// The size of the blocks a download is sent in.
const BLOCK: usize = 64 * 1024;

/// Write `body` into `file`, a block at a time, refusing one over [`MAX_UPLOAD`] bytes. The file
/// is removed when the body does not arrive whole.
async fn receive(body: Body, file: &std::path::Path) -> Result<(), ApiError> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let fail = |e: std::io::Error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            format!("writing the upload: {e}"),
        )
    };
    let result = async {
        let mut out = tokio::fs::File::create(file).await.map_err(fail)?;
        let mut stream = body.into_data_stream();
        let mut received = 0usize;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| ApiError::bad_request(format!("the upload stopped part way: {e}")))?;
            received += chunk.len();
            if received > MAX_UPLOAD {
                return Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    format!("an asset may be at most {MAX_UPLOAD} bytes"),
                ));
            }
            out.write_all(&chunk).await.map_err(fail)?;
        }
        out.flush().await.map_err(fail)?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(file).await;
    }
    result
}

/// Stream `body` into a fresh file in the vault's `.vault/incoming`, answering its path.
async fn incoming(w: &Arc<Writer>, body: Body) -> Result<std::path::PathBuf, ApiError> {
    let file = blocking(w.clone(), |w| w.incoming_file()).await?;
    receive(body, &file).await?;
    Ok(file)
}

/// `POST /api/assets?filename=<name>`.
pub async fn upload(
    State(w): Shared,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> Result<Response, ApiError> {
    let filename = q
        .filename
        .ok_or_else(|| ApiError::bad_request("name the file with ?filename="))?;
    let file = incoming(&w, body).await?;
    let asset = blocking(w, move |w| w.put_asset_file(&filename, &file)).await?;
    Ok((StatusCode::CREATED, Json(asset_json(&asset))).into_response())
}

/// `PUT /api/assets/{id}?filename=<name>`: store the body as asset `id`, an id the client chose,
/// so a device keeps the id of an asset it stored offline when it syncs (D19, D20). Sending the
/// same file again answers 200; anything else already under the id is 409 `id_taken`.
pub async fn put(
    State(w): Shared,
    Path(id): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> Result<Response, ApiError> {
    let filename = q
        .filename
        .ok_or_else(|| ApiError::bad_request("name the file with ?filename="))?;
    let file = incoming(&w, body).await?;
    let (existed, asset) = blocking(w, move |w| {
        let existed = w.asset_info(&id).is_ok();
        Ok::<_, writer::Error>((existed, w.put_asset_file_with_id(&id, &filename, &file)?))
    })
    .await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(asset_json(&asset))).into_response())
}

/// A body of `len` bytes read from `file` a block at a time.
fn file_body(file: std::fs::File, len: u64) -> Body {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::from_std(file).take(len);
    let stream = futures_util::stream::unfold(file, |mut file| async move {
        let mut buf = vec![0u8; BLOCK];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(Bytes::from(buf)), file))
            }
            Err(e) => Some((Err(e), file)),
        }
    });
    Body::from_stream(stream)
}

/// The milliseconds since the Unix epoch in a ULID: when the asset was uploaded.
fn ulid_ms(id: &str) -> Option<u64> {
    const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    id.get(..10)?.bytes().try_fold(0u64, |acc, b| {
        let d = CROCKFORD
            .iter()
            .position(|&c| c == b.to_ascii_uppercase())?;
        Some(acc * 32 + d as u64)
    })
}

/// What a file is, by its name: `image`, `pdf`, `audio`, `video` or `other`.
fn kind_of(filename: &str) -> &'static str {
    let ty = content_type(filename);
    if ty == "application/pdf" {
        "pdf"
    } else if let Some(k) = ["image", "audio", "video"]
        .into_iter()
        .find(|k| ty.starts_with(&format!("{k}/")))
    {
        k
    } else {
        "other"
    }
}

fn summary_json(s: &AssetSummary) -> Value {
    let mut v = asset_json(&s.asset);
    v["kind"] = json!(kind_of(&s.asset.filename));
    v["content_type"] = json!(content_type(&s.asset.filename));
    v["uploaded_ms"] = json!(ulid_ms(&s.asset.id));
    v["used_by"] = json!(s.used_by());
    v["notes"] = json!(s.notes);
    v["trashed"] = json!(s.trashed);
    v["types"] = json!(s.types);
    v
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// `image`, `pdf` or `other` (neither).
    kind: Option<String>,
    /// Only assets nothing uses.
    unused: Option<bool>,
    /// Words in the file name, or in a PDF's text.
    q: Option<String>,
    /// `uploaded` (the default), `name`, `size`, `kind` or `used`.
    sort: Option<String>,
    /// `asc` or `desc`. Uploaded sorts newest first by default, everything else ascending.
    dir: Option<String>,
    /// The `next` of the previous page.
    cursor: Option<String>,
    limit: Option<usize>,
}

/// `GET /api/assets[?kind=&unused=&q=&sort=&dir=&cursor=&limit=]`. With no parameters, every
/// asset oldest first. Each asset carries its kind, upload time and how many notes, trashed
/// notes and types use it; `unused` totals the assets nothing uses in the whole library, and
/// `counts` how many of the assets the search finds each filter would show.
pub async fn list(State(w): Shared, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let summaries = blocking(w.clone(), |w| w.asset_summaries()).await?;
    let unused: Vec<&AssetSummary> = summaries.iter().filter(|s| s.used_by() == 0).collect();
    let unused_json = json!({
        "count": unused.len(),
        "size": unused.iter().map(|s| s.asset.size).sum::<u64>(),
    });
    let plain = q.kind.is_none()
        && q.unused.is_none()
        && q.q.is_none()
        && q.sort.is_none()
        && q.cursor.is_none()
        && q.limit.is_none();
    let text =
        q.q.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase);
    let mut found = summaries;
    if let Some(t) = text {
        found = blocking(w, move |w| {
            Ok::<_, writer::Error>(
                found
                    .into_iter()
                    .filter(|s| {
                        s.asset.filename.to_lowercase().contains(&t)
                            || (kind_of(&s.asset.filename) == "pdf"
                                && w.asset_text_holds(&s.asset.id, &t))
                    })
                    .collect(),
            )
        })
        .await?;
    }
    // How many of what the search found each filter would show.
    let mut counts = serde_json::Map::new();
    counts.insert("all".into(), json!(found.len()));
    for k in ["image", "pdf", "other"] {
        let n = found
            .iter()
            .filter(|s| matches_kind(&s.asset.filename, k))
            .count();
        counts.insert(k.into(), json!(n));
    }
    let unused_found = found.iter().filter(|s| s.used_by() == 0).count();
    counts.insert("unused".into(), json!(unused_found));
    let mut picked: Vec<AssetSummary> = found
        .into_iter()
        .filter(|s| {
            q.kind
                .as_deref()
                .is_none_or(|k| matches_kind(&s.asset.filename, k))
        })
        .filter(|s| q.unused != Some(true) || s.used_by() == 0)
        .collect();
    if !plain {
        sort_summaries(&mut picked, q.sort.as_deref(), q.dir.as_deref());
    }
    let total = picked.len();
    let start = q
        .cursor
        .as_deref()
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(0)
        .min(total);
    let end = q.limit.map_or(total, |l| (start + l.max(1)).min(total));
    let next = (end < total).then(|| end.to_string());
    Ok(Json(json!({
        "assets": picked[start..end].iter().map(summary_json).collect::<Vec<_>>(),
        "total": total,
        "next": next,
        "unused": unused_json,
        "counts": counts,
    })))
}

/// Whether the file is of `kind` as a filter means it: `other` is neither an image nor a PDF,
/// and an empty kind or `all` is anything.
fn matches_kind(filename: &str, kind: &str) -> bool {
    let k = kind_of(filename);
    match kind {
        "image" | "pdf" => k == kind,
        "other" => k != "image" && k != "pdf",
        _ => true,
    }
}

fn sort_summaries(list: &mut [AssetSummary], sort: Option<&str>, dir: Option<&str>) {
    let sort = sort.unwrap_or("uploaded");
    let desc = match dir {
        Some("desc") => true,
        Some("asc") => false,
        _ => sort == "uploaded",
    };
    let name = |s: &AssetSummary| s.asset.filename.to_lowercase();
    list.sort_by(|a, b| {
        let o = match sort {
            "name" => name(a).cmp(&name(b)),
            "size" => a.asset.size.cmp(&b.asset.size),
            "kind" => kind_of(&a.asset.filename)
                .cmp(kind_of(&b.asset.filename))
                .then_with(|| ext(&a.asset.filename).cmp(&ext(&b.asset.filename))),
            "used" => a.used_by().cmp(&b.used_by()),
            _ => std::cmp::Ordering::Equal,
        };
        let o = o.then_with(|| a.asset.id.cmp(&b.asset.id));
        if desc { o.reverse() } else { o }
    });
}

fn ext(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// `GET /api/assets/{id}/usage`: the asset, and every note, trashed note and type using it.
pub async fn usage(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (asset, usage) = blocking(w, move |w| {
        let asset = w.asset_info(&id)?;
        Ok::<_, writer::Error>((asset, w.asset_usage(&id)?))
    })
    .await?;
    let s = AssetSummary {
        notes: usage.notes.len(),
        trashed: usage.trashed.len(),
        types: usage.types.len(),
        asset,
    };
    let mut body = summary_json(&s);
    body["notes"] = usage
        .notes
        .iter()
        .map(|n| {
            let how: Vec<Value> = n
                .how
                .iter()
                .map(|h| json!({ "field": h.field, "via": h.via }))
                .collect();
            json!({
                "id": n.note,
                "type": n.type_name,
                "title": n.title,
                "archived": n.archived,
                "how": how,
            })
        })
        .collect();
    body["trashed"] = usage
        .trashed
        .iter()
        .map(|t| json!({ "id": t.id, "type": t.type_name, "title": t.title }))
        .collect();
    body["types"] = json!(usage.types);
    Ok(Json(body))
}

#[derive(Deserialize)]
pub struct RenameBody {
    filename: String,
}

/// `PATCH /api/assets/{id}` with `{"filename": …}`: rename the file. Notes name assets by id,
/// so none changes.
pub async fn rename(
    State(w): Shared,
    Path(id): Path<String>,
    Json(b): Json<RenameBody>,
) -> Result<Json<Value>, ApiError> {
    let asset = blocking(w, move |w| w.rename_asset(&id, &b.filename)).await?;
    Ok(Json(asset_json(&asset)))
}

#[derive(Deserialize, Default)]
pub struct CleanupBody {
    #[serde(default)]
    dry_run: bool,
    /// Only these, of the assets nothing uses: the ones a dry run listed.
    ids: Option<Vec<String>>,
}

/// `POST /api/assets/cleanup` with `{"dry_run": bool, "ids": [...]}`: delete, in one commit,
/// every asset nothing uses (or those of them `ids` names) into the asset trash.
pub async fn cleanup(
    State(w): Shared,
    body: Option<Json<CleanupBody>>,
) -> Result<Json<Value>, ApiError> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let dry_run = b.dry_run;
    let assets = blocking(w, move |w| w.cleanup_assets(b.ids.as_deref(), dry_run)).await?;
    Ok(Json(json!({
        "dry_run": dry_run,
        "count": assets.len(),
        "size": assets.iter().map(|a| a.size).sum::<u64>(),
        "assets": assets.iter().map(asset_json).collect::<Vec<_>>(),
        "purge_days": writer::ASSET_TRASH_DAYS,
    })))
}

fn trashed_json(t: &TrashedAsset) -> Value {
    let mut v = asset_json(&t.asset);
    v["deleted_ms"] = json!(t.deleted_ms);
    v["purge_ms"] = json!(t.purge_ms);
    v
}

/// `GET /api/assets/trash`: deleted assets that can still be restored.
pub async fn trash(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let list = blocking(w, |w| Ok::<_, writer::Error>(w.asset_trash())).await?;
    Ok(Json(json!({
        "assets": list.iter().map(trashed_json).collect::<Vec<_>>(),
        "purge_days": writer::ASSET_TRASH_DAYS,
    })))
}

#[derive(Deserialize)]
pub struct RestoreBody {
    ids: Vec<String>,
}

/// `POST /api/assets/trash/restore` with `{"ids": [...]}`: bring deleted assets back. Each id
/// that cannot be restored is listed under `failed` with why.
pub async fn restore(
    State(w): Shared,
    Json(b): Json<RestoreBody>,
) -> Result<Json<Value>, ApiError> {
    let (restored, failed) = blocking(w, move |w| {
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        for id in &b.ids {
            match w.restore_asset(id) {
                Ok(a) => restored.push(asset_json(&a)),
                Err(e) => failed.push(json!({ "id": id, "error": e.to_string() })),
            }
        }
        Ok::<_, writer::Error>((restored, failed))
    })
    .await?;
    Ok(Json(json!({ "restored": restored, "failed": failed })))
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
    let (asset, part, file) = blocking(w, move |w| {
        let asset = w.asset_info(&id)?;
        let part = range
            .as_deref()
            .map_or(Part::Whole, |r| parse_range(r, asset.size));
        let file = match part {
            Part::Whole => Some(w.asset_file(&id, 0)?.1),
            Part::Range(start, _) => Some(w.asset_file(&id, start)?.1),
            Part::Unsatisfiable => None,
        };
        Ok::<_, writer::Error>((asset, part, file))
    })
    .await?;
    let size = asset.size;
    let mut res = match (part, file) {
        (Part::Whole, Some(file)) => {
            let mut res = file_body(file, size).into_response();
            res.headers_mut()
                .insert(header::CONTENT_LENGTH, HeaderValue::from(size));
            res
        }
        (Part::Range(start, end), Some(file)) => {
            let len = end - start + 1;
            let mut res = (StatusCode::PARTIAL_CONTENT, file_body(file, len)).into_response();
            res.headers_mut()
                .insert(header::CONTENT_LENGTH, HeaderValue::from(len));
            if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}")) {
                res.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            res
        }
        _ => {
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
pub(crate) enum Part {
    /// The whole file: no header, several ranges, or one that does not parse.
    Whole,
    /// Bytes `start..=end`, inside the file.
    Range(u64, u64),
    /// A range that starts past the end of the file.
    Unsatisfiable,
}

/// Read a `Range` header for a file of `size` bytes (RFC 9110, section 14.1.2). Only one range
/// of bytes is served on its own.
pub(crate) fn parse_range(header: &str, size: u64) -> Part {
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
/// them, unless `unlink` is set: then they lose their references to it in the same commit. A
/// note in the trash naming it always refuses, as `asset_in_trash`. The file goes to the asset
/// trash, from which `POST /api/assets/trash/restore` brings it back.
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
    body["trashed"] = json!(true);
    body["purge_days"] = json!(writer::ASSET_TRASH_DAYS);
    body["notes"] = saved
        .iter()
        .map(|s| json!({ "id": s.id, "type": s.type_name, "version": s.version }))
        .collect();
    Ok(Json(body))
}

/// The media type served for a file name. HTML is served as plain text.
pub(crate) fn content_type(filename: &str) -> &'static str {
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
pub(crate) fn percent_encode(s: &str) -> String {
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
