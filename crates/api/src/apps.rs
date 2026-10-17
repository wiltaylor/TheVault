//! The app builds the server hands out (`--clients <dir>`): `GET /api/clients` lists them from
//! the folder's `manifest.json`, and `GET /api/clients/{filename}` downloads one. Both are public,
//! so a phone's browser or Obtainium reaches them before the app is installed or paired: the
//! builds carry no secrets. `GET /api/clients/latest/{kind}` downloads the newest build of a kind
//! at an address that never changes, for an updater that tracks one URL.
//!
//! Only files the manifest lists are served, so a name that tries to leave the folder, or any
//! file the manifest does not name, is 404. A download carries its SHA-256 as the `ETag`, and
//! answers one `Range` the way an asset download does.
//!
//! `GET /api/clients/update/{target}/{arch}/{current_version}` is the desktop app's updater
//! endpoint, in the format the Tauri updater reads: 204 when nothing newer is published, else the
//! newer signed AppImage's `{version, notes, pub_date, url, signature}`.
//!
//! [`watch`] notices the manifest changing, as `vault clients publish` rewrites it while the
//! server runs, and publishes `clients.updated` on the live events, which `/api/events` streams
//! and the push dispatcher sends to each paired device.

use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use clients::{Entry, Kind};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{ClientBuild, LiveEvent};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use writer::Writer;

use crate::ApiError;
use crate::assets::{Part, parse_range, percent_encode};
use crate::devices::{qr_svg, server_url_from};

/// The clients folder, or `None` when the server hands out no builds, as an app's own embedded
/// server does.
pub struct Folder(pub Option<PathBuf>);

type Shared = State<Arc<Folder>>;

/// How much of a file each body chunk carries.
const CHUNK: usize = 64 * 1024;

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such client build")
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
}

async fn manifest(folder: &Folder) -> Result<clients::Manifest, ApiError> {
    let Some(dir) = folder.0.clone() else {
        return Ok(clients::Manifest::default());
    };
    tokio::task::spawn_blocking(move || clients::read(&dir))
        .await
        .map_err(internal)?
        .map_err(internal)
}

/// `GET /api/clients`: every published build, with its download `path`, its absolute `url` as
/// the request reached the server, and for an APK the QR code of that URL as an SVG image.
/// `enabled` is false when the server has no clients folder at all.
pub async fn list(State(folder): Shared, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let manifest = manifest(&folder).await?;
    let base = server_url_from(&headers).map(|u| u.trim_end_matches('/').to_owned());
    let clients: Vec<Value> = manifest
        .clients
        .iter()
        .map(|e| {
            let mut out = json!(e);
            let path = format!("/api/clients/{}", e.filename);
            if let Some(base) = &base {
                let url = format!("{base}{path}");
                if e.kind == Kind::Apk.as_str()
                    && let Ok(svg) = qr_svg(&url)
                {
                    out["qr_svg"] = json!(svg);
                }
                out["url"] = json!(url);
            }
            out["path"] = json!(path);
            out
        })
        .collect();
    // An app's own embedded server hands out nothing, and says so, so its pages leave the
    // downloads out rather than explain how to publish.
    Ok(Json(
        json!({ "clients": clients, "enabled": folder.0.is_some() }),
    ))
}

/// `GET /api/clients/{filename}`: one build's bytes.
pub async fn download(
    State(folder): Shared,
    Path(filename): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(dir) = folder.0.clone() else {
        return Err(not_found());
    };
    let found = tokio::task::spawn_blocking(move || clients::find(&dir, &filename))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    let (entry, path) = found.ok_or_else(not_found)?;
    send(entry, path, &headers).await
}

#[derive(Deserialize)]
pub struct LatestQuery {
    arch: Option<String>,
}

/// `GET /api/clients/latest/{kind}[?arch=]`: the build of `kind` (`apk`, `appimage`, `deb` or
/// `rpm`) for `arch`, or for the kind's usual architecture.
pub async fn latest(
    State(folder): Shared,
    Path(kind): Path<String>,
    Query(q): Query<LatestQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let kind = Kind::parse(&kind).ok_or_else(not_found)?;
    let Some(dir) = folder.0.clone() else {
        return Err(not_found());
    };
    let manifest = manifest(&folder).await?;
    let entry = clients::latest(&manifest, kind, q.arch.as_deref())
        .cloned()
        .ok_or_else(not_found)?;
    let filename = entry.filename.clone();
    let found = tokio::task::spawn_blocking(move || clients::find(&dir, &filename))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    let (entry, path) = found.ok_or_else(not_found)?;
    send(entry, path, &headers).await
}

/// `GET /api/clients/update/{target}/{arch}/{current_version}`: the Tauri updater's check. 204
/// when the server publishes no signed release AppImage for `target` and `arch` newer than
/// `current_version`; else the updater's JSON, whose `url` downloads the AppImage itself and
/// whose `signature` is the one published beside it.
pub async fn update(
    State(folder): Shared,
    Path((target, arch, current)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let manifest = manifest(&folder).await?;
    let Some(entry) = clients::update_for(&manifest, &target, &arch, &current) else {
        return Ok(no_update());
    };
    let path = format!("/api/clients/{}", percent_encode(&entry.filename));
    let Some(base) = server_url_from(&headers) else {
        return Ok(no_update());
    };
    let body = json!({
        "version": entry.version,
        "notes": format!("TheVault {}, published by your server", entry.version),
        "pub_date": clients::rfc3339(entry.published),
        "url": format!("{}{path}", base.trim_end_matches('/')),
        "signature": entry.signature,
    });
    let mut res = Json(body).into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

fn no_update() -> Response {
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}

/// What [`watch`] last saw of the manifest file: its modification time, size and inode. A
/// publish renames a new manifest into place, so each one changes at least the inode.
type Stamp = Option<(Option<SystemTime>, u64, u64)>;

fn stamp(dir: &std::path::Path) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(dir.join(clients::MANIFEST)).ok()?;
    Some((meta.modified().ok(), meta.len(), meta.ino()))
}

/// The builds of `manifest`, as `clients.updated` names them.
fn builds(manifest: &clients::Manifest) -> Vec<ClientBuild> {
    manifest
        .clients
        .iter()
        .map(|e| ClientBuild {
            kind: e.kind.clone(),
            arch: e.arch.clone(),
            version: e.version.clone(),
            build: e.build.clone(),
        })
        .collect()
}

/// Looks at the clients folder's manifest every `every`, and publishes `clients.updated` on
/// `writer`'s live events whenever the builds it lists change, whoever wrote it: `vault clients
/// publish`, `just publish-clients`, or a hand edit. Looking costs one `stat`; the manifest is
/// read only when the file changed. A manifest that does not parse, half-written by hand, is
/// read again on the next look. It runs on the current Tokio runtime until the runtime stops.
pub fn watch(writer: Arc<Writer>, dir: PathBuf, every: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let read = |dir: PathBuf| async move {
            tokio::task::spawn_blocking(move || (stamp(&dir), clients::read(&dir)))
                .await
                .ok()
        };
        let (mut seen, mut listed) = match read(dir.clone()).await {
            Some((stamp, Ok(manifest))) => (stamp, manifest),
            Some((_, Err(e))) => {
                eprintln!("clients: {e}");
                (None, clients::Manifest::default())
            }
            None => (None, clients::Manifest::default()),
        };
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            let d = dir.clone();
            let Ok(now) = tokio::task::spawn_blocking(move || stamp(&d)).await else {
                continue;
            };
            if now == seen {
                continue;
            }
            let Some((now, manifest)) = read(dir.clone()).await else {
                continue;
            };
            let manifest = match manifest {
                Ok(manifest) => manifest,
                Err(e) => {
                    eprintln!("clients: {e}");
                    continue;
                }
            };
            seen = now;
            if manifest != listed {
                writer.state().events().publish(LiveEvent::ClientsUpdated {
                    clients: builds(&manifest),
                });
                listed = manifest;
            }
        }
    })
}

/// Whether an `If-None-Match` or `If-Range` value names `etag`.
fn names_etag(value: Option<&HeaderValue>, etag: &str) -> bool {
    value
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
}

/// Stream `path`, all of it or the one range the request asks for.
async fn send(entry: Entry, path: PathBuf, headers: &HeaderMap) -> Result<Response, ApiError> {
    let kind = Kind::parse(&entry.kind).ok_or_else(not_found)?;
    let etag = format!("\"{}\"", entry.sha256);
    let mut res = if names_etag(headers.get(header::IF_NONE_MATCH), &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut file = tokio::fs::File::open(&path).await.map_err(internal)?;
        let size = file.metadata().await.map_err(internal)?.len();
        // A range is served only while the file is still the one the client started on.
        let range_ok = headers
            .get(header::IF_RANGE)
            .is_none_or(|v| names_etag(Some(v), &etag));
        let part = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
            Some(r) if range_ok => parse_range(r, size),
            _ => Part::Whole,
        };
        let (status, start, len) = match part {
            Part::Whole => (StatusCode::OK, 0, size),
            Part::Range(start, end) => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
            Part::Unsatisfiable => {
                let mut res = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                if let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}")) {
                    res.headers_mut().insert(header::CONTENT_RANGE, v);
                }
                return Ok(res);
            }
        };
        if start > 0 {
            file.seek(SeekFrom::Start(start)).await.map_err(internal)?;
        }
        let mut res = (status, Body::from_stream(chunks(file, len))).into_response();
        let h = res.headers_mut();
        if let Part::Range(start, end) = part
            && let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))
        {
            h.insert(header::CONTENT_RANGE, v);
        }
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(kind.content_type()),
        );
        if let Ok(v) = HeaderValue::from_str(&format!(
            "attachment; filename=\"{0}\"; filename*=UTF-8''{1}",
            entry.filename,
            percent_encode(&entry.filename)
        )) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
        res
    };
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(res)
}

/// The next `len` bytes of `file`, a chunk at a time.
fn chunks(
    file: tokio::fs::File,
    len: u64,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    futures_util::stream::unfold((file, len), |(mut file, left)| async move {
        if left == 0 {
            return None;
        }
        let mut buf = vec![0u8; CHUNK.min(usize::try_from(left).unwrap_or(CHUNK))];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), (file, left - n as u64)))
            }
            Err(e) => Some((Err(e), (file, 0))),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_etags_in_conditional_headers() {
        let etag = "\"abc\"";
        assert!(names_etag(Some(&HeaderValue::from_static("\"abc\"")), etag));
        assert!(names_etag(
            Some(&HeaderValue::from_static("\"x\", \"abc\"")),
            etag
        ));
        assert!(names_etag(Some(&HeaderValue::from_static("*")), etag));
        assert!(!names_etag(
            Some(&HeaderValue::from_static("\"abd\"")),
            etag
        ));
        assert!(!names_etag(None, etag));
    }
}
