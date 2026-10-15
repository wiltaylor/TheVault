//! Serves the built web client from a folder (D18). Paths outside `/api` that name no file get
//! `index.html`, so the client's own routes load on a refresh. The client is public: it holds no
//! data, and every `/api` route checks credentials on its own (D11).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Answers a request no `/api` route matched, with the file under `root` it names.
pub async fn serve(root: Arc<PathBuf>, req: Request) -> Response {
    let method = req.method().clone();
    let path = req.uri().path();
    if path == "/api" || path.starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": "not_found", "message": "no such endpoint" })),
        )
            .into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Some(rel) = relative(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || read(&root, &rel)).await {
        Ok(Some((name, bytes))) => file_response(&name, bytes, method == Method::HEAD),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The request path as a path under the root, or `None` when it tries to leave the root.
fn relative(path: &str) -> Option<PathBuf> {
    let mut rel = PathBuf::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        if segment == "." || segment == ".." || segment.contains('\\') || segment.contains('\0') {
            return None;
        }
        rel.push(segment);
    }
    Some(rel)
}

/// The file at `rel`, or `index.html` when `rel` names no file and does not look like one.
fn read(root: &Path, rel: &Path) -> Option<(String, Vec<u8>)> {
    let file = root.join(rel);
    if file.is_file() {
        let name = rel.to_string_lossy().into_owned();
        return std::fs::read(&file).ok().map(|b| (name, b));
    }
    let looks_like_file = rel
        .file_name()
        .is_some_and(|n| n.to_string_lossy().contains('.'));
    if looks_like_file {
        return None;
    }
    std::fs::read(root.join("index.html"))
        .ok()
        .map(|b| ("index.html".to_owned(), b))
}

fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn file_response(name: &str, bytes: Vec<u8>, head: bool) -> Response {
    // Vite puts content-hashed files under assets/; everything else may change in place.
    let cache = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let len = bytes.len();
    let body = if head {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut res = Response::new(body);
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(name)),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_paths_that_leave_the_root() {
        assert_eq!(relative("/"), Some(PathBuf::new()));
        assert_eq!(relative("/assets/a.js"), Some(PathBuf::from("assets/a.js")));
        assert_eq!(relative("/../etc/passwd"), None);
        assert_eq!(relative("/a/./b"), None);
        assert_eq!(relative("/a\\..\\b"), None);
    }
}
