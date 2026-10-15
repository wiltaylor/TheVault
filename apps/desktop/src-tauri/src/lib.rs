//! TheVault's Linux desktop app (D18, D21, D26).
//!
//! The app runs the server stack in-process on a local vault in the XDG data folder,
//! `$XDG_DATA_HOME/thevault/vault` ([`embedded::Embedded`]), so it works with no connection
//! (D21). The webview reaches it through the `vault` custom URI scheme with no network port:
//! `/api` goes to the embedded server, `/_desktop` to the app itself, and every other path serves
//! the bundled `web/dist`, so the web client stays same-origin and unchanged.
//!
//! Until the device is paired the window shows the setup page, which pairs with the home server
//! by password or pairing code ([`device`]). Afterwards [`sync_loop`] keeps the local vault and
//! the home server in step, and a small badge injected into every page shows where sync stands,
//! including the "update the app" refusal when the server is newer (D27).
//!
//! `thevault --capture` opens the capture dialog, in the running app when there is one: a key
//! the user binds in their desktop runs it (D12).

mod device;
mod sync_loop;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use embedded::{Config, Embedded};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tauri::http::{Method, Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use device::{Keyring, PairRequest};
use sync_loop::{SyncLoop, Trigger};

/// The custom URI scheme the webview loads the app from.
const SCHEME: &str = "vault";

/// Where the webview reaches [`SCHEME`] on Linux.
const ORIGIN: &str = "vault://localhost";

/// The page that pairs the device.
const SETUP_PATH: &str = "/_desktop/setup";

const SETUP_PAGE: &str = include_str!("setup.html");

/// Runs in every page the window loads: the sync badge and the capture hook.
const PAGE_SCRIPT: &str = include_str!("page.js");

/// The command-line flag that opens the capture dialog.
const CAPTURE_FLAG: &str = "--capture";

/// The app's state, managed by Tauri.
struct App {
    embedded: Arc<Embedded>,
    sync: SyncLoop,
    /// The folder of the local vault and the device file.
    dir: PathBuf,
    /// Whether `--capture` asked for the capture dialog and the page has not opened it yet.
    capture: AtomicBool,
}

pub fn run() {
    let capture = std::env::args().any(|a| a == CAPTURE_FLAG);
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            show_main(app);
            if args.iter().any(|a| a == CAPTURE_FLAG) {
                request_capture(app);
            }
        }))
        .register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(serve(&app, request).await);
            });
        })
        .setup(move |app| {
            let dir = app.path().data_dir()?.join("thevault");
            let vault = dir.join("vault");
            std::fs::create_dir_all(&vault)?;
            let embedded = Arc::new(Embedded::open(Config {
                vault,
                web: None,
                secret_key: None,
            })?);
            let paired = device::load(&dir)?;
            let start = if paired.is_some() { "/" } else { SETUP_PATH };
            let sync = SyncLoop::start(embedded.clone(), paired, Box::new(Keyring));
            app.manage(App {
                embedded,
                sync,
                dir,
                capture: AtomicBool::new(capture),
            });
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::CustomProtocol(format!("{ORIGIN}{start}").parse()?),
            )
            .title("TheVault")
            .inner_size(1200.0, 800.0)
            .initialization_script(PAGE_SCRIPT)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("TheVault failed to start");
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Marks the capture dialog as asked for, and tells the page, which opens it.
fn request_capture(app: &AppHandle) {
    if let Some(state) = app.try_state::<App>() {
        state.capture.store(true, Ordering::SeqCst);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval("window.__thevaultCapture && window.__thevaultCapture()");
    }
}

/// Answers one request on [`SCHEME`].
async fn serve(app: &AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(state) = app.try_state::<App>() else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let path = request.uri().path().to_owned();
    if path == "/api" || path.starts_with("/api/") {
        return api(&state, request).await;
    }
    if path == "/_desktop" || path.starts_with("/_desktop/") {
        return desktop(&state, request).await;
    }
    asset(app, &path)
}

/// `/api` from the embedded server. A write that succeeded wakes the sync loop.
async fn api(state: &App, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let writes = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let (parts, body) = state
        .embedded
        .handle(request.map(axum::body::Body::from))
        .await
        .into_parts();
    if writes && parts.status.is_success() {
        state.sync.trigger(Trigger::LocalWrite);
    }
    match body.collect().await {
        Ok(body) => Response::from_parts(parts, body.to_bytes().to_vec()),
        Err(_) => status(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// The app's own routes: the setup page, pairing, sync status and the capture hook.
async fn desktop(state: &App, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path();
    match (request.method().clone(), path) {
        (Method::GET, SETUP_PATH) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(SETUP_PAGE.as_bytes().to_vec())
            .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR)),
        (Method::GET, "/_desktop/status") => {
            json_response(StatusCode::OK, &json!(state.sync.status()))
        }
        (Method::POST, "/_desktop/sync") => {
            state.sync.trigger(Trigger::Now);
            json_response(StatusCode::ACCEPTED, &json!({}))
        }
        (Method::POST, "/_desktop/capture") => {
            let open = state.capture.swap(false, Ordering::SeqCst);
            json_response(StatusCode::OK, &json!({ "open": open }))
        }
        (Method::POST, "/_desktop/pair") => pair(state, request.body()).await,
        _ => status(StatusCode::NOT_FOUND),
    }
}

/// `POST /_desktop/pair`: pair with the home server, then start syncing with it.
async fn pair(state: &App, body: &[u8]) -> Response<Vec<u8>> {
    let req: PairRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let dir = state.dir.clone();
    // Pairing and the keyring block, so they run on a thread of their own.
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let paired = device::pair(&dir, &Keyring, &req).and_then(|device| {
            use device::TokenStore;
            Keyring.get(&device.device_id).map(|token| (device, token))
        });
        let _ = tx.send(paired);
    });
    match rx.await {
        Ok(Ok((device, token))) => {
            let out = json!({ "device_id": device.device_id, "device_name": device.device_name });
            state.sync.trigger(Trigger::Paired(device, token));
            json_response(StatusCode::OK, &out)
        }
        Ok(Err(message)) => error(StatusCode::BAD_REQUEST, &message),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "pairing stopped"),
    }
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

fn json_response(code: StatusCode, value: &Value) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "application/json")
        .body(value.to_string().into_bytes())
        .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
}

fn error(code: StatusCode, message: &str) -> Response<Vec<u8>> {
    json_response(code, &json!({ "message": message }))
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    let mut res = Response::new(Vec::new());
    *res.status_mut() = code;
    res
}
