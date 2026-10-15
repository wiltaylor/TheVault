//! The app's Rust core: one in-process server per process, shared by the webview and by the
//! Kotlin code that runs with no webview (D17, D18).
//!
//! The Tauri app and the background workers run in the same Android process when the app is
//! open, and each reaches the core: the app through [`open`] in the plugin's setup, the workers
//! through JNI. Both open the same vault folder, `<data dir>/vault`, and get the same [`Core`], so
//! the vault is only ever opened once per process.
//!
//! The device token lives in the Android Keystore (D2). Kotlin hands the core the pairing it
//! holds with [`Core::set_link`]; the core keeps it in memory only.
//!
//! Sync (D19, D20) runs in three ways, one round at a time:
//! - in the foreground, when the app opens and after each local write: [`Core::kick`] wakes a
//!   thread that waits for writes to settle and then runs a round;
//! - in the background, from WorkManager's periodic worker, through [`Core::sync_now`];
//! - for assets, on demand: [`Core::fetch_asset`] when a note shows an asset the device does not
//!   hold, and [`Core::note_opened`] when a note is opened, so its assets are prefetched next time.
//!
//! Device capture (D7, D17) hands events to the device runtime [`Core::device`] opens, whose
//! device module reaches the phone through the Kotlin bridge.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use embedded::device::{DeviceConfig, DeviceData, DeviceRuntime};
use embedded::{Config, Embedded};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sync::{AssetPolicy, Report};

use crate::notify::{self, PushTarget};
use crate::share::{self, Shared};

/// How long the foreground loop waits after a write for more writes before it syncs.
const SETTLE: Duration = Duration::from_millis(1500);

/// The asset cache size when the app's setting is missing (D19).
pub const DEFAULT_CACHE_MB: u64 = 512;

/// How many recently opened notes have their assets prefetched.
const RECENT_NOTES: usize = 50;

/// The vault folder inside the app's data folder. Kotlin builds the same path.
pub fn vault_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("vault")
}

/// A paired device's link to its home server, as Kotlin stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub server_url: String,
    pub token: String,
    pub device_id: String,
    pub device_name: String,
    /// The asset cache size in MiB.
    #[serde(default = "default_cache_mb")]
    pub cache_mb: u64,
}

fn default_cache_mb() -> u64 {
    DEFAULT_CACHE_MB
}

impl Link {
    fn remote(&self) -> sync::Remote {
        sync::Remote {
            url: self.server_url.clone(),
            token: self.token.clone(),
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
        }
    }

    fn assets(&self) -> AssetPolicy {
        AssetPolicy::OnDemand {
            cache_bytes: self.cache_mb.max(1) * 1024 * 1024,
            recent: RECENT_NOTES,
        }
    }
}

/// How the last round went.
#[derive(Debug, Clone, Default)]
struct Last {
    at_ms: i64,
    error: Option<String>,
}

/// The core of one process.
pub struct Core {
    vault: PathBuf,
    server: Embedded,
    link: Mutex<Option<Link>>,
    /// Held for the whole of a round, so rounds never overlap.
    round: Mutex<()>,
    /// Wakes the foreground loop; `None` until the first kick starts it.
    kicks: Mutex<Option<Sender<()>>>,
    last: Mutex<Option<Last>>,
    /// The device runtime and the device id it opened with, once Kotlin opens it.
    device: Mutex<Option<(String, Arc<DeviceRuntime>)>>,
}

/// The platform the device runtime runs as, which names its device file.
pub const PLATFORM: &str = "android";

static CORE: Mutex<Option<Arc<Core>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// The process's core for the vault folder `vault`, opening it on first use. Asking for another
/// folder closes the old core once its last user lets go: a process has one vault.
pub fn open(vault: &Path) -> anyhow::Result<Arc<Core>> {
    let mut current = lock(&CORE);
    if let Some(core) = current.as_ref()
        && core.vault == vault
    {
        return Ok(core.clone());
    }
    std::fs::create_dir_all(vault)?;
    let server = Embedded::open(Config {
        vault: vault.to_owned(),
        web: None,
        secret_key: None,
    })?;
    let core = Arc::new(Core {
        vault: vault.to_owned(),
        server,
        link: Mutex::new(None),
        round: Mutex::new(()),
        kicks: Mutex::new(None),
        last: Mutex::new(None),
        device: Mutex::new(None),
    });
    *current = Some(core.clone());
    Ok(core)
}

/// The process's core, once [`open`] has run.
pub fn current() -> Option<Arc<Core>> {
    lock(&CORE).clone()
}

impl Core {
    pub fn vault(&self) -> &Path {
        &self.vault
    }

    /// The in-process server the webview talks to.
    pub fn server(&self) -> &Embedded {
        &self.server
    }

    /// The device runtime of device `id`, opened with `data` unless one is open for `id` already.
    /// Another id, after pairing, replaces it; the queue lives in `state.db`, so queued events
    /// carry over.
    pub fn device(
        &self,
        id: &str,
        data: impl FnOnce() -> Arc<dyn DeviceData>,
    ) -> anyhow::Result<Arc<DeviceRuntime>> {
        let mut device = lock(&self.device);
        if let Some((open_id, runtime)) = device.as_ref()
            && open_id == id
        {
            return Ok(runtime.clone());
        }
        let runtime = Arc::new(self.server.device(DeviceConfig {
            id: id.to_owned(),
            platform: PLATFORM.to_owned(),
            data: data(),
        })?);
        *device = Some((id.to_owned(), runtime.clone()));
        Ok(runtime)
    }

    /// The device runtime [`Core::device`] opened.
    pub fn runtime(&self) -> anyhow::Result<Arc<DeviceRuntime>> {
        lock(&self.device)
            .as_ref()
            .map(|(_, runtime)| runtime.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("the device runtime is not open: call VaultCore.openDevice")
            })
    }

    /// Set the pairing Kotlin holds, or clear it when the device is not paired.
    pub fn set_link(&self, link: Option<Link>) {
        *lock(&self.link) = link;
    }

    pub fn link(&self) -> Option<Link> {
        lock(&self.link).clone()
    }

    /// Run one round now, waiting for any round already running. It blocks: never call it from
    /// inside an async runtime.
    pub fn sync_now(&self) -> anyhow::Result<Report> {
        let link = self.linked()?;
        let _round = lock(&self.round);
        let result = self
            .server
            .sync_engine(link.remote(), link.assets())
            .and_then(|engine| engine.sync());
        *lock(&self.last) = Some(Last {
            at_ms: now_ms(),
            error: result.as_ref().err().map(ToString::to_string),
        });
        Ok(result?)
    }

    /// Ask the foreground loop for a round once writes settle. Nothing happens while unpaired.
    pub fn kick(self: &Arc<Self>) {
        if self.link().is_none() {
            return;
        }
        let mut kicks = lock(&self.kicks);
        if let Some(tx) = kicks.as_ref()
            && tx.send(()).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel::<()>();
        let core = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("vault-sync".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    while rx.recv_timeout(SETTLE).is_ok() {}
                    let Some(core) = core.upgrade() else { return };
                    if let Err(e) = core.sync_now() {
                        eprintln!("sync failed: {e}");
                    }
                }
            })
            .expect("starting the sync thread");
        let _ = tx.send(());
        *kicks = Some(tx);
    }

    /// Store what another app shared as a fleeting note, then ask for a round. Works offline:
    /// the note waits in the queue. Returns the note's id.
    pub fn share(self: &Arc<Self>, shared: &Shared) -> anyhow::Result<String> {
        let id = share::import(self.server.writer(), shared)?;
        self.kick();
        Ok(id)
    }

    /// Fetch asset `id` from the home server into the local vault. `Ok(false)` when the device
    /// is not paired or the server never listed the asset. It blocks.
    pub fn fetch_asset(&self, id: &str) -> anyhow::Result<bool> {
        let Some(link) = self.link() else {
            return Ok(false);
        };
        if self.server.queue().asset(id)?.is_none() {
            return Ok(false);
        }
        let engine = self.server.sync_engine(link.remote(), link.assets())?;
        engine.fetch_asset(id)?;
        Ok(true)
    }

    /// Note `id` was opened: remember it for the prefetch and fetch its missing assets. It
    /// blocks.
    pub fn note_opened(&self, id: &str) -> anyhow::Result<()> {
        match self.link() {
            Some(link) => self
                .server
                .sync_engine(link.remote(), link.assets())?
                .note_opened(id)?,
            None => self.server.queue().opened(id)?,
        }
        Ok(())
    }

    /// The notes waiting for the home server, oldest first, each with its type, its fields and
    /// the assets its fields name: `[{id, type, fields, assets: [{id, filename, size}]}]`.
    pub fn queued(&self) -> anyhow::Result<Value> {
        let writer = self.server.writer();
        let mut out = Vec::new();
        for p in self.server.queue().pending()? {
            let Ok(note) = writer.read(&p.note_id) else {
                out.push(json!({ "id": p.note_id, "deleted": true }));
                continue;
            };
            let doc = writer::doc_to_json(&note.doc);
            let mut assets = Vec::new();
            for v in doc["fields"]
                .as_object()
                .into_iter()
                .flat_map(|f| f.values())
            {
                let ids: Vec<&str> = match v {
                    Value::String(s) => vec![s.as_str()],
                    Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                    _ => Vec::new(),
                };
                for asset in ids.into_iter().filter_map(|i| writer.asset_info(i).ok()) {
                    assets.push(json!({
                        "id": asset.id,
                        "filename": asset.filename,
                        "size": asset.size,
                    }));
                }
            }
            out.push(json!({
                "id": note.id,
                "type": note.type_name,
                "fields": doc["fields"],
                "blocks": doc["blocks"],
                "assets": assets,
            }));
        }
        Ok(Value::Array(out))
    }

    fn linked(&self) -> anyhow::Result<Link> {
        self.link()
            .ok_or_else(|| anyhow::anyhow!("this device is not paired"))
    }

    /// The home server's unread notifications, newest first (D9). It blocks.
    pub fn unread_notifications(&self) -> anyhow::Result<Vec<Value>> {
        notify::unread(&self.linked()?)
    }

    /// Mark notification `id` read on the home server; `Ok(false)` when the server no longer
    /// has it. It blocks.
    pub fn mark_read(&self, id: &str) -> anyhow::Result<bool> {
        notify::mark_read(&self.linked()?, id)
    }

    /// Register the UnifiedPush endpoint with the home server, or clear it with `None` (D4). It
    /// blocks.
    pub fn set_push(&self, target: Option<&PushTarget>) -> anyhow::Result<()> {
        notify::set_push(&self.linked()?, target)
    }

    /// Whether the device is paired, how many notes wait, and how the last round went.
    pub fn status(&self) -> anyhow::Result<Value> {
        let last = lock(&self.last).clone();
        Ok(json!({
            "paired": self.link().is_some(),
            "pending": self.server.queue().pending()?.len(),
            "last_sync_ms": last.as_ref().map(|l| l.at_ms),
            "last_error": last.and_then(|l| l.error),
        }))
    }
}

/// A round's report as JSON.
pub fn report_json(r: &Report) -> Value {
    json!({
        "pushed": r.pushed,
        "merged": r.merged,
        "conflicts": r.conflicts.iter().map(|c| json!({
            "note_id": c.note_id,
            "copy_id": c.copy_id,
            "reason": c.reason,
        })).collect::<Vec<_>>(),
        "pulled": r.pulled,
        "assets_fetched": r.assets_fetched,
        "assets_evicted": r.assets_evicted,
    })
}
