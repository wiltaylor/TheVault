//! TheVault's Android app (D14, D18, D26).
//!
//! The app runs the server stack in-process on a local vault in its data folder
//! ([`embedded::Embedded`]). The webview reaches it through the `vault` custom URI scheme, so no
//! network port is opened: `/api` goes to the embedded server and every other path serves the
//! bundled `web/dist`, so the web client stays same-origin and unchanged.

use std::sync::Arc;

use embedded::{Config, Embedded};
use http_body_util::BodyExt;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

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
        .setup(|app| {
            let vault = app.path().app_data_dir()?.join("vault");
            std::fs::create_dir_all(&vault)?;
            let server = Embedded::open(Config {
                vault,
                web: None,
                secret_key: None,
            })?;
            app.manage(Arc::new(server));
            WebviewWindowBuilder::new(app, "main", WebviewUrl::CustomProtocol(ORIGIN.parse()?))
                .title("TheVault")
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("TheVault failed to start");
}

/// Answers one request on [`SCHEME`]: `/api` from the embedded server, the rest from `web/dist`.
async fn serve(app: &AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path();
    if path != "/api" && !path.starts_with("/api/") {
        return asset(app, path);
    }
    let Some(server) = app.try_state::<Arc<Embedded>>() else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let server = server.inner().clone();
    let (parts, body) = server
        .handle(request.map(axum::body::Body::from))
        .await
        .into_parts();
    match body.collect().await {
        Ok(body) => Response::from_parts(parts, body.to_bytes().to_vec()),
        Err(_) => status(StatusCode::INTERNAL_SERVER_ERROR),
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

fn status(code: StatusCode) -> Response<Vec<u8>> {
    let mut res = Response::new(Vec::new());
    *res.status_mut() = code;
    res
}
