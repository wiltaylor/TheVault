//! The webview's request bodies, carried over Tauri IPC.
//!
//! Android's webview serves a custom scheme through `WebViewClient.shouldInterceptRequest`, whose
//! request carries no body: a POST, PUT or PATCH on the `vault` scheme reaches the embedded server
//! empty. So in the app the web client sends every request that has a body through these commands
//! instead (`web/src/api.ts` picks them when the page script sets `__thevaultApiOverIpc`), and the
//! embedded server answers it as it would over the scheme.
//!
//! A text body (JSON) travels inline. Bytes, an asset upload of up to 512 MiB, travel as base64
//! chunks appended to an upload file in the app's cache, and stream from there into the request,
//! so no single IPC message, and no copy in memory, holds the whole file.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use tauri::http::{Method, Request, StatusCode, header};
use tauri_plugin_vault_device::core::Core;

/// Runs in every page before its own scripts: tells the web client to send bodies over IPC.
pub const PAGE_SCRIPT: &str =
    "Object.defineProperty(window, '__thevaultApiOverIpc', { value: true });";

/// One request from the web client, as `web/src/api.ts` sends it.
#[derive(Debug, Deserialize)]
pub struct IpcRequest {
    pub method: String,
    /// The path and query, `/api/...`.
    pub path: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// A text body, sent as it is.
    #[serde(default)]
    pub text: Option<String>,
    /// A body of bytes, appended to the upload of this id by [`Uploads::append`].
    #[serde(default)]
    pub upload: Option<u64>,
}

/// The embedded server's answer. The web client reads every answer as text.
#[derive(Debug, Serialize, PartialEq)]
pub struct IpcResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl IpcResponse {
    fn error(code: StatusCode, message: &str) -> IpcResponse {
        let error = if code == StatusCode::BAD_REQUEST {
            "bad_request"
        } else {
            "internal"
        };
        IpcResponse {
            status: code.as_u16(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::json!({ "error": error, "message": message }).to_string(),
        }
    }
}

/// The request bodies of bytes being sent, each a file in `dir` until its request takes it.
pub struct Uploads {
    dir: PathBuf,
    next: AtomicU64,
    open: Mutex<HashMap<u64, std::fs::File>>,
}

impl Uploads {
    /// Uploads kept in `dir`. What an earlier run left there is deleted.
    pub fn new(dir: PathBuf) -> Uploads {
        let _ = std::fs::remove_dir_all(&dir);
        Uploads {
            dir,
            next: AtomicU64::new(1),
            open: Mutex::new(HashMap::new()),
        }
    }

    fn path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("upload-{id}"))
    }

    /// Start an empty upload and return its id.
    pub fn start(&self) -> anyhow::Result<u64> {
        std::fs::create_dir_all(&self.dir)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let file = std::fs::File::create(self.path(id))?;
        lock(&self.open).insert(id, file);
        Ok(id)
    }

    /// Append the base64 `chunk` to upload `id`.
    pub fn append(&self, id: u64, chunk: &str) -> anyhow::Result<()> {
        let bytes = STANDARD.decode(chunk)?;
        let mut open = lock(&self.open);
        let file = open
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no upload {id}"))?;
        file.write_all(&bytes)?;
        Ok(())
    }

    /// Close upload `id` and hand its file to the request; the file is deleted once read.
    fn take(&self, id: u64) -> anyhow::Result<Upload> {
        let file = lock(&self.open)
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("no upload {id}"))?;
        file.sync_all()?;
        drop(file);
        Ok(Upload(self.path(id)))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An upload file, deleted when the request is done with it.
struct Upload(PathBuf);

impl Upload {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Answer `req` from `core`'s embedded server, then ask for a sync round after a write.
pub async fn forward(core: &Arc<Core>, uploads: &Uploads, req: IpcRequest) -> IpcResponse {
    let Ok(method) = Method::from_bytes(req.method.as_bytes()) else {
        return IpcResponse::error(StatusCode::BAD_REQUEST, "the method is not valid");
    };
    if req.path != "/api" && !req.path.starts_with("/api/") && !req.path.starts_with("/api?") {
        return IpcResponse::error(StatusCode::BAD_REQUEST, "only /api is served over IPC");
    }
    let mut upload = None;
    let body = match (req.text, req.upload) {
        (Some(text), None) => axum::body::Body::from(text),
        (None, Some(id)) => {
            let file = match uploads.take(id) {
                Ok(file) => file,
                Err(e) => return IpcResponse::error(StatusCode::BAD_REQUEST, &e.to_string()),
            };
            let opened = match tokio::fs::File::open(file.path()).await {
                Ok(opened) => opened,
                Err(e) => return IpcResponse::error(StatusCode::BAD_REQUEST, &e.to_string()),
            };
            upload = Some(file);
            axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(opened))
        }
        (None, None) => axum::body::Body::empty(),
        (Some(_), Some(_)) => {
            return IpcResponse::error(StatusCode::BAD_REQUEST, "a request has one body");
        }
    };
    let mut builder = Request::builder().method(method.clone()).uri(&req.path);
    for (name, value) in &req.headers {
        // The body is complete here; a length the page computed is not needed.
        if !name.eq_ignore_ascii_case(header::CONTENT_LENGTH.as_str()) {
            builder = builder.header(name, value);
        }
    }
    let request = match builder.body(body) {
        Ok(request) => request,
        Err(e) => return IpcResponse::error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let (parts, body) = core.server().handle(request).await.into_parts();
    drop(upload);
    let body = match body.collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => {
            return IpcResponse::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the answer could not be read",
            );
        }
    };
    if crate::is_write(&method) && parts.status.is_success() {
        core.kick();
    }
    IpcResponse {
        status: parts.status.as_u16(),
        headers: parts
            .headers
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect(),
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const ARTICLE: &str =
        "label = \"Article\"\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";

    fn request(method: &str, path: &str, text: Option<Value>, upload: Option<u64>) -> IpcRequest {
        let mut headers = vec![("accept".to_owned(), "application/json".to_owned())];
        if text.is_some() {
            headers.push(("content-type".to_owned(), "application/json".to_owned()));
        }
        if upload.is_some() {
            headers.push((
                "content-type".to_owned(),
                "application/octet-stream".to_owned(),
            ));
        }
        IpcRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            headers,
            text: text.map(|v| v.to_string()),
            upload,
        }
    }

    fn json_of(res: &IpcResponse) -> Value {
        serde_json::from_str(&res.body).unwrap_or(Value::Null)
    }

    /// The bodies that Android's custom scheme drops arrive whole over IPC: a JSON note write, a
    /// JSON update against its version, and an asset upload in base64 chunks.
    #[test]
    fn request_bodies_round_trip_through_ipc_to_the_embedded_server() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::write(vault.join("types/article.wcl"), ARTICLE).unwrap();
        let core = tauri_plugin_vault_device::core::open(&vault).unwrap();
        let uploads = Uploads::new(dir.path().join("uploads"));

        tauri::async_runtime::block_on(async {
            let send = |req| forward(&core, &uploads, req);

            let note = json!({ "note": { "fields": { "title": "probe" }, "blocks": [] } });
            let res = send(request("POST", "/api/notes?type=article", Some(note), None)).await;
            assert_eq!(res.status, 201, "{}", res.body);
            let created = json_of(&res);
            let id = created["id"].as_str().unwrap().to_owned();
            assert_eq!(created["note"]["fields"]["title"], "probe", "{created}");

            let update = json!({
                "note": { "fields": { "title": "probe, edited" }, "blocks": [] },
                "base_version": created["version"],
            });
            let path = format!("/api/notes/{id}");
            let res = send(request("PUT", &path, Some(update), None)).await;
            assert_eq!(res.status, 200, "{}", res.body);
            let res = send(request("GET", &path, None, None)).await;
            assert_eq!(json_of(&res)["note"]["fields"]["title"], "probe, edited");

            // Bytes travel as base64 chunks; every byte value survives.
            let file: Vec<u8> = (0..300_000u32).map(|i| (i % 256) as u8).collect();
            let upload = uploads.start().unwrap();
            for chunk in file.chunks(64 * 1024) {
                uploads.append(upload, &STANDARD.encode(chunk)).unwrap();
            }
            let path = "/api/assets?filename=probe.bin";
            let res = send(request("POST", path, None, Some(upload))).await;
            assert_eq!(res.status, 201, "{}", res.body);
            let asset = json_of(&res);
            assert_eq!(asset["size"], file.len(), "{asset}");
            assert!(
                !uploads.path(upload).exists(),
                "the upload file is deleted once sent"
            );
            let asset_id = asset["id"].as_str().unwrap();
            let res = core
                .server()
                .handle(
                    Request::get(format!("/api/assets/{asset_id}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await;
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(&bytes[..], &file[..]);

            // A spent upload, or a path outside /api, is refused.
            let res = send(request("POST", path, None, Some(upload))).await;
            assert_eq!(res.status, 400, "{}", res.body);
            let res = send(request("POST", "/index.html", Some(json!({})), None)).await;
            assert_eq!(res.status, 400, "{}", res.body);
        });
    }
}
