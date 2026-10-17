//! The AppImage updating itself from the paired server (D27), with the Tauri updater.
//!
//! The update thread ([`Updates::run`]) checks soon after start, every [`EVERY`] after that, and
//! at once when the server announces a new build (`clients.updated` on its event stream, which
//! [`crate::live`] hands to [`Updates::wake`]). A check asks the server's updater endpoint; a
//! newer signed AppImage is downloaded in the background and its signature verified against the
//! public key built into the app (`plugins.updater.pubkey` in `tauri.conf.json`). It then waits,
//! [`Ready`], and the window and the tray say "Update ready — restart to update". Restarting
//! installs it; with "Install updates on quit" on, quitting installs it too.
//!
//! When the server has no signed AppImage newer than the app, and for a deb or rpm install, the
//! check falls back to [`crate::update`]'s offer: the server's download, else GitHub's release.
//!
//! A debug build reads `THEVAULT_TEST_APPIMAGE`, the file to treat as the running AppImage, and
//! `THEVAULT_TEST_UPDATER_PUBKEY`, the public key to verify with, so the S6 tests can run the
//! whole check and download against a build they signed with a throwaway key.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::update::{self, Install, Release};

/// How long after start the first check runs, and how often it runs after that.
pub const FIRST: Duration = Duration::from_secs(10);
pub const EVERY: Duration = Duration::from_secs(4 * 60 * 60);

/// How long the updater waits for the server, for the check and the whole download.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// A downloaded, verified update waiting to be installed.
pub struct Ready {
    pub version: String,
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
}

/// What the update check knows, for `/_desktop/status`.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// How the app was installed: `appimage`, `package` or `binary`.
    pub install: Install,
    /// How many checks have finished since the app started.
    pub checks: u64,
    /// The version downloaded and waiting for a restart.
    pub ready: Option<String>,
    /// Why the last check or download failed.
    pub error: Option<String>,
}

/// The update check's state, shared by the update thread, the window's routes and the tray.
pub struct Updates {
    install: Install,
    /// The AppImage the updater replaces, when the app runs as one.
    appimage: Option<PathBuf>,
    /// A newer release to offer as a download.
    found: Mutex<Option<Release>>,
    ready: Mutex<Option<Ready>>,
    error: Mutex<Option<String>>,
    checks: AtomicU64,
    wake: Mutex<Option<Sender<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The file to treat as the running AppImage: `$APPIMAGE`, or in a debug build
/// `THEVAULT_TEST_APPIMAGE`.
fn appimage_path() -> Option<PathBuf> {
    if cfg!(debug_assertions)
        && let Some(path) = std::env::var_os("THEVAULT_TEST_APPIMAGE").filter(|p| !p.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("APPIMAGE")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// The public key a debug build verifies with instead of the built-in one.
fn test_pubkey() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("THEVAULT_TEST_UPDATER_PUBKEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
}

impl Updates {
    pub fn new() -> Updates {
        let appimage = appimage_path();
        let exe = std::env::current_exe().unwrap_or_default();
        Updates {
            install: update::install_kind(appimage.as_deref().map(|p| p.as_os_str()), &exe),
            appimage,
            found: Mutex::new(None),
            ready: Mutex::new(None),
            error: Mutex::new(None),
            checks: AtomicU64::new(0),
            wake: Mutex::new(None),
        }
    }

    pub fn install(&self) -> Install {
        self.install
    }

    /// The release the window offers: the downloaded one waiting for a restart, else a newer
    /// release to download.
    pub fn offered(&self) -> Option<Release> {
        if let Some(ready) = lock(&self.ready).as_ref() {
            return Some(Release {
                version: ready.version.clone(),
                url: ready.update.download_url.to_string(),
                source: "server",
            });
        }
        lock(&self.found).clone()
    }

    pub fn ready_version(&self) -> Option<String> {
        lock(&self.ready).as_ref().map(|r| r.version.clone())
    }

    pub fn status(&self) -> Status {
        Status {
            install: self.install,
            checks: self.checks.load(Ordering::SeqCst),
            ready: self.ready_version(),
            error: lock(&self.error).clone(),
        }
    }

    /// Asks the update thread to check now.
    pub fn wake(&self) {
        if let Some(tx) = lock(&self.wake).as_ref() {
            let _ = tx.send(());
        }
    }

    /// Installs the update waiting, if there is one. Answers the version installed. The new
    /// AppImage runs the next time the app starts.
    pub fn install_ready(&self) -> Result<Option<String>, String> {
        let Some(ready) = lock(&self.ready).take() else {
            return Ok(None);
        };
        ready
            .update
            .install(&ready.bytes)
            .map_err(|e| format!("installing TheVault {}: {e}", ready.version))?;
        Ok(Some(ready.version))
    }

    /// Starts the update thread. `github` is where the fallback offer asks, and `on` says
    /// whether checking is turned on now. `changed` runs after each check that changed what is
    /// ready, for the tray.
    pub fn start(
        self: &Arc<Self>,
        app: AppHandle,
        github: String,
        on: impl Fn(&AppHandle) -> Option<Option<String>> + Send + 'static,
        changed: impl Fn(&AppHandle) + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        *lock(&self.wake) = Some(tx);
        let this = self.clone();
        std::thread::Builder::new()
            .name("update-check".to_owned())
            .spawn(move || this.run(app, github, rx, on, changed))
            .expect("the update thread starts");
    }

    fn run(
        &self,
        app: AppHandle,
        github: String,
        wake: Receiver<()>,
        on: impl Fn(&AppHandle) -> Option<Option<String>>,
        changed: impl Fn(&AppHandle),
    ) {
        let mut wait = FIRST;
        loop {
            match wake.recv_timeout(wait) {
                Ok(()) => {
                    // A burst of announcements is one check.
                    while wake.try_recv().is_ok() {}
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            wait = EVERY;
            // `None` when checking is off; else the paired server's URL, if paired.
            let Some(server) = on(&app) else {
                continue;
            };
            let before = self.ready_version();
            self.check(&app, server.as_deref(), &github);
            self.checks.fetch_add(1, Ordering::SeqCst);
            if self.ready_version() != before {
                changed(&app);
            }
        }
    }

    /// One check: the server's signed AppImage first, then the offer.
    fn check(&self, app: &AppHandle, server: Option<&str>, github: &str) {
        let current = app.package_info().version.to_string();
        *lock(&self.error) = None;
        if let (Some(endpoint), Some(appimage)) = (
            update::updater_endpoint(self.install, server),
            self.appimage.clone(),
        ) {
            match tauri::async_runtime::block_on(
                self.self_update(app, &endpoint, appimage, &current),
            ) {
                Ok(true) => {
                    *lock(&self.found) = None;
                    return;
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("thevault: update: {e}");
                    *lock(&self.error) = Some(e);
                }
            }
        }
        match update::check_preferring_server(server, github, &current) {
            Ok(release) => *lock(&self.found) = release,
            Err(e) => {
                eprintln!("thevault: update check: {e}");
                lock(&self.error).get_or_insert(e);
            }
        }
    }

    /// Asks `endpoint` for a newer signed AppImage, and downloads and verifies it when it is not
    /// the one already waiting. Answers whether an update is ready.
    async fn self_update(
        &self,
        app: &AppHandle,
        endpoint: &str,
        appimage: PathBuf,
        current: &str,
    ) -> Result<bool, String> {
        let url = endpoint
            .parse()
            .map_err(|e| format!("the updater endpoint {endpoint}: {e}"))?;
        let mut builder = app
            .updater_builder()
            .endpoints(vec![url])
            .map_err(|e| e.to_string())?
            .executable_path(appimage)
            .timeout(CHECK_TIMEOUT);
        if let Some(key) = test_pubkey() {
            builder = builder.pubkey(key);
        }
        let updater = builder.build().map_err(|e| e.to_string())?;
        let Some(found) = updater
            .check()
            .await
            .map_err(|e| format!("checking {endpoint}: {e}"))?
        else {
            // Nothing newer on the server; anything downloaded before is kept.
            return Ok(lock(&self.ready).is_some());
        };
        let ready = self.ready_version();
        if !update::should_download(current, ready.as_deref(), &found.version) {
            return Ok(ready.is_some());
        }
        let mut download = found.clone();
        download.timeout = Some(DOWNLOAD_TIMEOUT);
        let bytes = download
            .download(|_, _| {}, || {})
            .await
            .map_err(|e| format!("downloading TheVault {}: {e}", found.version))?;
        *lock(&self.ready) = Some(Ready {
            version: found.version.clone(),
            update: found,
            bytes,
        });
        Ok(true)
    }
}

/// Installs the waiting update and starts the new version.
pub fn restart_to_update(app: &AppHandle) -> Result<(), String> {
    let Some(state) = app.try_state::<crate::App>() else {
        return Ok(());
    };
    match state.updates.install_ready()? {
        Some(_) => app.restart(),
        None => Ok(()),
    }
}
