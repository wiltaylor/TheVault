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
//! The desktop extras (D9, D12, D27): a tray icon with the home server's unread count and a
//! menu ([`tray`]); server notifications mirrored on the desktop and closed when read anywhere
//! ([`live`]); start on login, minimised ([`settings`]); a global capture hotkey through the
//! portal ([`hotkey`]); `thevault://note/<id>` links ([`links`]); an AppImage's application
//! menu entry and icons ([`app_menu`]), after it installed itself in `~/Applications`
//! ([`app_install`]); and the update check
//! ([`update`]), with which an AppImage updates itself from the paired server
//! ([`self_update`]).
//!
//! `thevault --capture` opens the capture dialog, in the running app when there is one: a key
//! the user binds in their desktop runs it where the portal cannot (D12). Closing the window
//! hides it in the tray; the tray's Quit ends the app.

mod app_install;
mod app_menu;
mod device;
mod hotkey;
mod links;
mod live;
mod notifier;
mod self_update;
mod settings;
mod sync_loop;
mod tray;
mod update;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use embedded::{Config, Embedded};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tauri::http::{Method, Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use device::{Keyring, PairRequest};
use live::Live;
use self_update::Updates;
use settings::{MINIMISED_FLAG, Settings};
use sync_loop::{SyncLoop, Trigger};
use tray::Tray;
use update::Release;

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
    app: AppHandle,
    embedded: Arc<Embedded>,
    sync: SyncLoop,
    /// The folder of the local vault and the device file.
    dir: PathBuf,
    /// Whether `--capture` asked for the capture dialog and the page has not opened it yet.
    capture: AtomicBool,
    live: Live,
    tray: Tray,
    settings: Mutex<Settings>,
    hotkey: Arc<Mutex<hotkey::Status>>,
    /// What the update check found, and the update downloaded and waiting.
    updates: Arc<Updates>,
}

impl App {
    fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    /// The newer release to offer, unless the user turned the check off or skipped it.
    fn offered_update(&self) -> Option<Release> {
        let settings = self.settings();
        self.updates.offered().filter(|r| {
            settings.update_check && settings.skipped_update.as_ref() != Some(&r.version)
        })
    }

    /// Changes the settings with `change`, saves them and applies them.
    fn change_settings(&self, app: &AppHandle, change: impl FnOnce(&mut Settings)) {
        let settings = {
            let mut s = lock(&self.settings);
            change(&mut s);
            s.clone()
        };
        if let Err(e) = settings::save(&self.dir, &settings) {
            eprintln!("thevault: {e}");
        }
        apply_autostart(app, &settings);
        apply_app_menu(app, &settings);
        self.tray.show_settings(&settings);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes or removes the autostart entry to match `settings`.
fn apply_autostart(app: &AppHandle, settings: &Settings) {
    let Ok(config) = app.path().config_dir() else {
        return;
    };
    if let Err(e) =
        settings::apply_autostart(&config, &settings::launcher_exe(), settings.autostart)
    {
        eprintln!("thevault: {e}");
    }
}

/// Installs or removes the AppImage's application menu entry to match `settings`.
fn apply_app_menu(app: &AppHandle, settings: &Settings) {
    if let Ok(data) = app.path().data_dir() {
        app_menu::apply(&data, settings.app_menu);
    }
}

/// What the live link needs from the app: the tray badge and the window.
struct LiveShell(AppHandle);

impl live::Shell for LiveShell {
    fn badge(&self, unread: u64) {
        if let Some(state) = self.0.try_state::<App>() {
            state.tray.set_unread(unread);
        }
    }

    fn open(&self, path: &str) {
        open_path(&self.0, path);
    }

    fn server_changed(&self, change: Option<sync::live::NoteChanged>) {
        if let Some(state) = self.0.try_state::<App>() {
            state.sync.trigger(Trigger::ServerChanged(change));
        }
    }

    fn clients_updated(&self) {
        if let Some(state) = self.0.try_state::<App>() {
            state.updates.wake();
        }
    }
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(app_install::VERSION_FLAG) {
        println!("TheVault {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if app_install::settle(&args) {
        return;
    }
    app_menu::name_program();
    let capture = args.iter().any(|a| a == CAPTURE_FLAG);
    let minimised = args
        .iter()
        .any(|a| a == MINIMISED_FLAG || a == "--minimized");
    let link = links::from_args(&args[1.min(args.len())..]);
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(link) = links::from_args(&args) {
                open_path(app, &link.path());
            } else if !args.iter().any(|a| a == MINIMISED_FLAG) {
                show_main(app);
            }
            if args.iter().any(|a| a == CAPTURE_FLAG) {
                show_main(app);
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
            let start = match (&paired, &link) {
                (None, _) => SETUP_PATH.to_owned(),
                (Some(_), Some(link)) => link.path(),
                (Some(_), None) => "/".to_owned(),
            };
            let (remote_tx, remote_rx) = tokio::sync::watch::channel(None);
            let sync = SyncLoop::start(embedded.clone(), paired, Box::new(Keyring), remote_tx);
            let settings = settings::load(&dir);
            let handle = app.handle().clone();
            apply_autostart(&handle, &settings);
            apply_app_menu(&handle, &settings);
            let tray = Tray::build(&handle, &settings);
            let live = Live::start(Arc::new(LiveShell(handle.clone())), remote_rx);
            let hotkey = Arc::new(Mutex::new(hotkey::Status::Starting));
            let updates = Arc::new(Updates::new());
            app.manage(App {
                app: handle.clone(),
                embedded,
                sync,
                dir,
                capture: AtomicBool::new(capture),
                live,
                tray,
                settings: Mutex::new(settings),
                hotkey: hotkey.clone(),
                updates: updates.clone(),
            });
            let window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::CustomProtocol(format!("{ORIGIN}{start}").parse()?),
            )
            .title("TheVault")
            .inner_size(1200.0, 800.0)
            .visible(!minimised)
            .initialization_script(PAGE_SCRIPT)
            .icon(app_menu::window_icon())?
            .build()?;
            window.on_window_event({
                let app = handle.clone();
                move |event| {
                    // With a tray to come back from, closing the window hides it.
                    if let WindowEvent::CloseRequested { api, .. } = event
                        && app.try_state::<App>().is_some_and(|s| s.tray.badge().tray)
                        && let Some(window) = app.get_webview_window("main")
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            });
            let capture_app = handle.clone();
            hotkey::start(hotkey, move || {
                show_main(&capture_app);
                request_capture(&capture_app);
            });
            start_update_checks(handle, &updates);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("TheVault failed to start")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                install_on_quit(app);
            }
        });
}

/// With "Install updates on quit" on, installs the update waiting, so the next start runs it.
fn install_on_quit(app: &AppHandle) {
    let Some(state) = app.try_state::<App>() else {
        return;
    };
    if !state.settings().install_on_quit {
        return;
    }
    match state.updates.install_ready() {
        Ok(Some(version)) => eprintln!("thevault: installed TheVault {version}; it runs next time"),
        Ok(None) => {}
        Err(e) => eprintln!("thevault: {e}"),
    }
}

/// What the tray menu's items do.
fn on_tray_menu(app: &AppHandle, id: &str) {
    let Some(state) = app.try_state::<App>() else {
        return;
    };
    match id {
        tray::CAPTURE => {
            show_main(app);
            request_capture(app);
        }
        tray::OPEN => show_main(app),
        tray::AUTOSTART => state.change_settings(app, |s| s.autostart = !s.autostart),
        tray::UPDATE_CHECK => state.change_settings(app, |s| s.update_check = !s.update_check),
        tray::INSTALL_ON_QUIT => {
            state.change_settings(app, |s| s.install_on_quit = !s.install_on_quit)
        }
        tray::APP_MENU => state.change_settings(app, |s| s.app_menu = !s.app_menu),
        tray::RESTART_TO_UPDATE => {
            if let Err(e) = self_update::restart_to_update(app) {
                eprintln!("thevault: {e}");
            }
        }
        tray::QUIT => app.exit(0),
        _ => {}
    }
}

/// Checks for a newer release soon after start, every few hours after that, and when the server
/// announces new builds, while the setting is on ([`self_update`]).
fn start_update_checks(app: AppHandle, updates: &Arc<Updates>) {
    let Some(github) = update::endpoint() else {
        return;
    };
    updates.start(
        app,
        github,
        |app| {
            let state = app.try_state::<App>()?;
            if !state.settings().update_check {
                return None;
            }
            // The paired server comes first; it is read each time, since the app may have
            // paired since the last check.
            Some(
                device::load(&state.dir)
                    .ok()
                    .flatten()
                    .map(|d| d.server_url),
            )
        },
        |app| {
            if let Some(state) = app.try_state::<App>() {
                state
                    .tray
                    .set_update_ready(state.updates.ready_version().as_deref());
            }
        },
    );
}

/// Shows the window at the web client's `path`.
fn open_path(app: &AppHandle, path: &str) {
    if let Some(window) = app.get_webview_window("main") {
        // The page navigates itself: `path` is a JSON string, so it cannot break out of the call.
        let target = json!(format!("{ORIGIN}{path}"));
        if let Err(e) = window.eval(format!("location.assign({target})")) {
            eprintln!("thevault: opening {path}: {e}");
        }
    }
    show_main(app);
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
    // The custom scheme answers with whole bodies, so an event stream that never ends could never
    // be answered: the web client is told at once and polls instead, and no stream to the home
    // server is held open for nothing.
    if request.uri().path() == "/api/events" {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &json!({ "error": "no_stream", "message": "the app polls instead of streaming events" }),
        );
    }
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
        (Method::GET, "/_desktop/status") => json_response(StatusCode::OK, &status_json(state)),
        (Method::POST, "/_desktop/update/skip") => {
            if let Some(release) = state.updates.offered() {
                state.change_settings(&state.app, |s| s.skipped_update = Some(release.version));
            }
            json_response(StatusCode::OK, &json!({}))
        }
        (Method::POST, "/_desktop/update/restart") => {
            match self_update::restart_to_update(&state.app) {
                Ok(()) => status(StatusCode::NOT_FOUND),
                Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
            }
        }
        (Method::POST, "/_desktop/update/open") => match state.offered_update() {
            Some(release) => {
                open_external(&release.url);
                json_response(StatusCode::OK, &json!({}))
            }
            None => status(StatusCode::NOT_FOUND),
        },
        (Method::POST, "/_desktop/installed/undo") => {
            state.change_settings(&state.app, |s| s.app_menu = false);
            app_install::clear_notice(&state.dir);
            json_response(StatusCode::OK, &json!({}))
        }
        (Method::POST, "/_desktop/installed/dismiss") => {
            app_install::clear_notice(&state.dir);
            json_response(StatusCode::OK, &json!({}))
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

/// `GET /_desktop/status`: the sync status, with the desktop extras beside it.
fn status_json(state: &App) -> Value {
    let mut out = json!(state.sync.status());
    out["notifications"] = json!(state.live.status());
    out["tray"] = json!(state.tray.badge());
    let ready = state.updates.ready_version();
    let mut update = json!(state.offered_update());
    if let Some(offer) = update.as_object_mut() {
        let is_ready =
            ready.is_some() && offer.get("version") == ready.as_ref().map(|v| json!(v)).as_ref();
        offer.insert("ready".to_owned(), json!(is_ready));
        offer.insert("install".to_owned(), json!(state.updates.install()));
    }
    out["update"] = update;
    out["updater"] = json!(state.updates.status());
    out["hotkey"] = json!(*lock(&state.hotkey));
    out["settings"] = json!(state.settings());
    out["installed"] = json!(app_install::notice(&state.dir));
    out
}

/// Opens `url` in the user's browser.
fn open_external(url: &str) {
    if let Err(e) = std::process::Command::new("xdg-open").arg(url).spawn() {
        eprintln!("thevault: opening {url}: {e}");
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
