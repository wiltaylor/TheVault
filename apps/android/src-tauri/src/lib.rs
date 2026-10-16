//! TheVault's Android app (D14, D18, D26).
//!
//! The app runs the server stack in-process on a local vault in its data folder
//! ([`embedded::Embedded`]). The webview reaches it through the `vault` custom URI scheme, so no
//! network port is opened: `/api` goes to the embedded server and every other path serves the
//! bundled `web/dist`, so the web client stays same-origin and unchanged.
//!
//! The device plugin opens that server as the process's [`Core`], which the Kotlin side shares
//! for background sync and the share target. The scheme handler feeds the sync loop (D19, D20):
//! a local write asks for a round, an asset the device does not hold is fetched from the home
//! server before it is answered, and an opened note is remembered so its assets are prefetched.
//!
//! Android's webview drops the body of a request on a custom scheme, so the web client sends
//! every request with a body through IPC instead ([`bridge`]).

mod bridge;

use std::sync::Arc;

use http_body_util::BodyExt;
use tauri::http::{Method, Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_vault_device::CoreFailure;
use tauri_plugin_vault_device::core::Core;

/// The custom URI scheme the webview loads the app from.
const SCHEME: &str = "vault";

/// Where the webview reaches [`SCHEME`]: Android's webview serves custom schemes as
/// `http://<scheme>.localhost`, Linux's as `<scheme>://localhost`.
#[cfg(any(target_os = "android", target_os = "windows"))]
const ORIGIN: &str = "http://vault.localhost/";
#[cfg(not(any(target_os = "android", target_os = "windows")))]
const ORIGIN: &str = "vault://localhost/";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_vault_device::init())
        .register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(serve(&app, request).await);
            });
        })
        .invoke_handler(tauri::generate_handler![
            api_request,
            api_upload_start,
            api_upload_append
        ])
        .setup(|app| {
            app.manage(bridge::Uploads::new(
                app.path().app_cache_dir()?.join("uploads"),
            ));
            WebviewWindowBuilder::new(app, "main", WebviewUrl::CustomProtocol(ORIGIN.parse()?))
                .title("TheVault")
                .initialization_script(bridge::PAGE_SCRIPT)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("TheVault failed to start");
}

/// Answers one `/api` request the web client sent over IPC, body and all ([`bridge`]).
#[tauri::command]
async fn api_request(
    app: AppHandle,
    request: bridge::IpcRequest,
) -> Result<bridge::IpcResponse, String> {
    let core = app
        .try_state::<Arc<Core>>()
        .ok_or("the vault is not open yet")?
        .inner()
        .clone();
    let uploads = app.state::<bridge::Uploads>();
    Ok(bridge::forward(&core, &uploads, request).await)
}

/// Starts the body of bytes of an IPC request and returns its id.
#[tauri::command]
async fn api_upload_start(uploads: State<'_, bridge::Uploads>) -> Result<u64, String> {
    uploads.start().map_err(|e| e.to_string())
}

/// Appends a base64 chunk to the body of bytes `id`.
#[tauri::command]
async fn api_upload_append(
    uploads: State<'_, bridge::Uploads>,
    id: u64,
    chunk: String,
) -> Result<(), String> {
    uploads.append(id, &chunk).map_err(|e| e.to_string())
}

/// Whether a successful request of `method` may have changed the vault, so sync should run.
fn is_write(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Answers one request on [`SCHEME`]: `/api` from the embedded server, the rest from `web/dist`.
async fn serve(app: &AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if let Some(failure) = app.try_state::<CoreFailure>() {
        return failure_page(&failure.0);
    }
    let path = request.uri().path();
    if path != "/api" && !path.starts_with("/api/") {
        return asset(app, path);
    }
    // The custom scheme answers with whole bodies, so an event stream that never ends could never
    // be answered: the web client is told at once and polls instead, and no stream to the home
    // server is held open for nothing.
    if path == "/api/events" {
        let mut res = Response::new(
            br#"{"error":"no_stream","message":"the app polls instead of streaming events"}"#
                .to_vec(),
        );
        *res.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        return res;
    }
    let Some(core) = app.try_state::<Arc<Core>>() else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let core = core.inner().clone();
    let method = request.method().clone();
    let path = path.to_owned();
    let retry = (method == Method::GET).then(|| request.clone());
    let mut res = call(&core, request).await;
    if method == Method::GET {
        if let (Some(id), Some(retry)) = (asset_id(&path), retry)
            && res.status() == StatusCode::NOT_FOUND
        {
            let fetched = {
                let (core, id) = (core.clone(), id.to_owned());
                off_runtime(move || core.fetch_asset(&id)).await
            };
            if matches!(fetched, Ok(true)) {
                res = call(&core, retry).await;
            }
        }
        if let Some(id) = note_id(&path)
            && res.status().is_success()
        {
            let (core, id) = (core.clone(), id.to_owned());
            std::thread::spawn(move || {
                if let Err(e) = core.note_opened(&id) {
                    eprintln!("prefetching the assets of note {id} failed: {e}");
                }
            });
        }
    } else if is_write(&method) && res.status().is_success() {
        core.kick();
    }
    res
}

/// One request to the in-process server, its body collected.
async fn call(core: &Core, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let (parts, body) = core
        .server()
        .handle(request.map(axum::body::Body::from))
        .await
        .into_parts();
    match body.collect().await {
        Ok(body) => Response::from_parts(parts, body.to_bytes().to_vec()),
        Err(_) => status(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Run blocking sync work on a thread of its own: its blocking HTTP client must never run, or be
/// dropped, on an async runtime's thread.
async fn off_runtime<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("the sync thread stopped")))
}

/// The asset a `GET /api/assets/{id}...` request reads.
fn asset_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/api/assets/")?.split('/').next()?;
    is_ulid(id).then_some(id)
}

/// The note a `GET /api/notes/{id}` request opens.
fn note_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/api/notes/")?;
    is_ulid(id).then_some(id)
}

fn is_ulid(s: &str) -> bool {
    s.len() == 26 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The bundled web client's file at `path`, or `index.html` for a client-side route: a path whose
/// last segment has no extension.
fn asset(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    let resolver = app.asset_resolver();
    let found = resolver.get(path.to_owned()).or_else(|| {
        let last = path.rsplit('/').next().unwrap_or_default();
        (!last.contains('.'))
            .then(|| resolver.get("index.html".to_owned()))
            .flatten()
    });
    let Some(asset) = found else {
        return status(StatusCode::NOT_FOUND);
    };
    let mut res = Response::builder().header(header::CONTENT_TYPE, asset.mime_type());
    if let Some(csp) = asset.csp_header() {
        res = res.header(header::CONTENT_SECURITY_POLICY, csp);
    }
    res.body(asset.bytes().to_vec())
        .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
}

/// The page every request gets when the local vault failed to open: the error, rather than an app
/// that cannot load or save anything.
fn failure_page(error: &str) -> Response<Vec<u8>> {
    let error = error
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>TheVault</title></head>\
         <body style=\"font-family: sans-serif; padding: 1.5rem\">\
         <h1>TheVault could not start</h1>\
         <p>The app could not open its local vault.</p><pre style=\"white-space: pre-wrap\">{error}</pre>\
         </body></html>"
    );
    let mut res = Response::new(body.into_bytes());
    *res.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    let mut res = Response::new(Vec::new());
    *res.status_mut() = code;
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_asset_and_note_ids_in_api_paths() {
        let id = "01J9Z3K8Q2W6E4R5T7Y8U9I0OP";
        assert_eq!(asset_id(&format!("/api/assets/{id}")), Some(id));
        assert_eq!(asset_id(&format!("/api/assets/{id}/thumb")), Some(id));
        assert_eq!(asset_id("/api/assets"), None);
        assert_eq!(note_id(&format!("/api/notes/{id}")), Some(id));
        assert_eq!(note_id(&format!("/api/notes/{id}/history")), None);
        assert_eq!(note_id("/api/notes/new"), None);
    }

    #[test]
    fn the_failure_page_shows_the_error_escaped() {
        let res = failure_page("opening /data/vault: <lock> not supported");
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = String::from_utf8(res.into_body()).unwrap();
        assert!(body.contains("TheVault could not start"));
        assert!(body.contains("opening /data/vault: &lt;lock&gt; not supported"));
    }
}
