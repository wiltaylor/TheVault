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
//! Sync (D19, D20) runs in these ways, one round at a time:
//! - in the foreground, when the app opens and after each local write: [`Core::kick`] wakes a
//!   thread that waits for writes to settle and then runs a round;
//! - in the foreground, when a note changes on the server: while Kotlin says the app is in front
//!   ([`Core::set_foreground`]), a thread holds the home server's `GET /api/events` open with
//!   the device token, reconnecting with a growing pause, and asks for a round a second after
//!   each `note.changed` the device does not hold yet, after each `device.settings` for this
//!   device, and on each (re)connect;
//! - in the background, from WorkManager's periodic worker, and from the prompt worker a
//!   `note.changed` or `device.settings` push schedules, through [`Core::sync_now`];
//! - for assets, on demand: [`Core::fetch_asset`] when a note shows an asset the device does not
//!   hold, and [`Core::note_opened`] when a note is opened, so its assets are prefetched next time.
//!
//! Every round first takes the device's settings from Settings > Devices on the home server
//! (D22, D23, D25), so a switch turned on or off reaches the device runtime by the next round,
//! and the `device.settings` event and push make that round come at once.
//!
//! Device capture (D7, D17) hands events to the device runtime [`Core::device`] opens, whose
//! device module reaches the phone through the Kotlin bridge. The runtime is handed to the
//! embedded server too, which lists the events that failed or were skipped for This device in
//! Settings > Devices. Each round ends by reporting the device events that failed for good to the
//! home server, one bell notification each ([`commands::report_failures`]). After each round the runtime queues
//! a `contact_changed` event for each linked `person` that changed, so the change reaches the
//! phone contact (D16). Commands from server scripts run through [`Core::run_commands`] (D22).
//!
//! An import of history ([`Core::import_history`]) reads the phone's messages, calls and contacts
//! again over a range of dates, and files each that has no note yet (D7).
//!
//! A share from another app is a `share_received` device event too: [`Core::share`] queues it and
//! runs it at once, before pairing as well, since its handler needs only the local vault.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use embedded::device::{DeviceConfig, DeviceData, DeviceRuntime, ImportReport, ImportRequest};
use embedded::{Config, Embedded, SyncPhase, SyncReport, SyncSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sync::live::{self, Seen};
use sync::{AssetPolicy, Report};

use crate::commands;
use crate::notify::{self, PushTarget};
use crate::share::{self, Outcome, Shared};

/// How long the foreground loop waits after a write for more writes before it syncs.
const SETTLE: Duration = Duration::from_millis(1500);

/// How long the foreground loop waits after a change on the server for more before it pulls.
const REMOTE_SETTLE: Duration = Duration::from_secs(1);

/// The first pause before the event stream reconnects, doubled after each failure up to
/// [`MAX_BACKOFF`].
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

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
#[derive(Debug, Clone)]
struct Last {
    at_ms: i64,
    error: Option<String>,
    /// What the error means for the user: offline, update required, revoked or another error;
    /// synced when the round succeeded.
    phase: SyncPhase,
    /// When a round last succeeded.
    ok_ms: Option<i64>,
}

/// The live event stream's state.
#[derive(Debug, Default)]
struct Live {
    /// Whether the app is in front, as Kotlin last said.
    foreground: bool,
    /// Whether the thread that follows the stream runs.
    running: bool,
    /// Whether the stream is open now.
    connected: bool,
}

/// The core of one process.
pub struct Core {
    vault: PathBuf,
    server: Embedded,
    link: Mutex<Option<Link>>,
    /// Held for the whole of a round, so rounds never overlap.
    round: Mutex<()>,
    /// Wakes the foreground loop with how long to wait for more; `None` until the first kick
    /// starts it.
    kicks: Mutex<Option<Sender<Duration>>>,
    live: Mutex<Live>,
    last: Mutex<Option<Last>>,
    /// Whether a round runs now.
    syncing: AtomicBool,
    /// The device runtime and the device id it opened with, once Kotlin opens it.
    device: Mutex<Option<(String, Arc<DeviceRuntime>)>>,
    /// Held while commands run, so a push and the periodic check never take one command twice.
    commands: Mutex<()>,
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
    crate::logging::init();
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
        live: Mutex::new(Live::default()),
        last: Mutex::new(None),
        syncing: AtomicBool::new(false),
        device: Mutex::new(None),
        commands: Mutex::new(()),
    });
    core.server
        .set_sync_source(Box::new(Source(Arc::downgrade(&core))));
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
        self.server.set_device(&runtime);
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
        // The embedded server sends the home server's own routes, such as Settings > Devices and
        // the notifications, to the same server.
        self.server.set_remote(link.as_ref().map(Link::remote));
        *lock(&self.link) = link;
    }

    pub fn link(&self) -> Option<Link> {
        lock(&self.link).clone()
    }

    /// Run one round now, waiting for any round already running: take the device's settings from
    /// the home server, sync, then queue the person changes for their phone contacts. It blocks:
    /// never call it from inside an async runtime.
    pub fn sync_now(&self) -> anyhow::Result<Report> {
        let link = self.linked()?;
        let _round = lock(&self.round);
        self.syncing.store(true, Ordering::SeqCst);
        if let Err(e) = self.refresh_settings(&link) {
            log::warn!("the device settings were not refreshed: {e:#}");
        }
        let result = self
            .server
            .sync_engine(link.remote(), link.assets())
            .and_then(|engine| engine.sync());
        let at_ms = now_ms();
        {
            let mut last = lock(&self.last);
            let ok_ms = match &result {
                Ok(_) => Some(at_ms),
                Err(_) => last.as_ref().and_then(|l| l.ok_ms),
            };
            let (phase, error) = match &result {
                Ok(_) => (SyncPhase::Synced, None),
                Err(e) => {
                    let (phase, message) = SyncPhase::of_error(e);
                    (phase, Some(message))
                }
            };
            *last = Some(Last {
                at_ms,
                error,
                phase,
                ok_ms,
            });
        }
        self.syncing.store(false, Ordering::SeqCst);
        let report = result?;
        if let Err(e) = self.people_to_contacts() {
            log::warn!("person changes not queued for the phone: {e:#}");
        }
        if let Err(e) = self.report_failures(&link) {
            log::warn!("failed device events not reported to the home server: {e:#}");
        }
        Ok(report)
    }

    /// Take the device's settings from the home server into the device runtime, when one is
    /// open. The settings stay as they were when the server no longer lists the device.
    fn refresh_settings(&self, link: &Link) -> anyhow::Result<()> {
        let Ok(runtime) = self.runtime() else {
            return Ok(());
        };
        if runtime.device_id() != link.device_id {
            return Ok(());
        }
        if let Some(s) = commands::settings(link)?.filter(Value::is_object) {
            runtime.set_settings(&s)?;
        }
        Ok(())
    }

    /// Report the device events that failed for good to the home server, when a runtime of this
    /// device is open.
    fn report_failures(&self, link: &Link) -> anyhow::Result<()> {
        let Ok(runtime) = self.runtime() else {
            return Ok(());
        };
        if runtime.device_id() != link.device_id {
            return Ok(());
        }
        commands::report_failures(link, &runtime)?;
        Ok(())
    }

    /// Queue the person changes the round brought for their phone contacts, and run them.
    fn people_to_contacts(&self) -> anyhow::Result<()> {
        let Ok(runtime) = self.runtime() else {
            return Ok(());
        };
        if runtime.queue_person_changes()? > 0 {
            runtime.run_pending()?;
        }
        Ok(())
    }

    /// Run the commands server scripts queued for this device, after taking its settings from
    /// the home server. Answers what ran, `[{ id, name, status, result }]`. It blocks.
    pub fn run_commands(&self) -> anyhow::Result<Value> {
        let link = self.linked()?;
        let runtime = self.runtime()?;
        let _one = lock(&self.commands);
        commands::run(&link, &runtime)
    }

    /// Import the phone's history as `request` says, `{ kinds, since, until }`: each message and
    /// call with no note yet runs as its device event now, so the device file's handlers decide
    /// the notes; one that has a note is left alone ([`DeviceRuntime::import_history`]).
    /// `progress` hears the report as it goes. Then ask for a round. It blocks.
    pub fn import_history(
        self: &Arc<Self>,
        request: &Value,
        progress: &mut dyn FnMut(&ImportReport),
    ) -> anyhow::Result<ImportReport> {
        let req = ImportRequest::from_json(request).map_err(anyhow::Error::msg)?;
        let report = self.runtime()?.import_history(&req, progress)?;
        self.kick();
        Ok(report)
    }

    /// Whether the vault holds the `message` or `call` note of `device_ref`.
    pub fn noted(&self, device_ref: &str) -> anyhow::Result<bool> {
        self.runtime()?.noted(device_ref)
    }

    /// Ask the foreground loop for a round once writes settle. Nothing happens while unpaired.
    pub fn kick(self: &Arc<Self>) {
        self.nudge(SETTLE);
    }

    /// Ask the foreground loop for a round at once: the sync icon's "Sync now". Nothing happens
    /// while unpaired.
    pub fn sync_soon(self: &Arc<Self>) {
        self.nudge(Duration::ZERO);
    }

    /// Where sync stands, for the web client's sync icon (`GET /api/app/sync`).
    pub fn sync_report(&self) -> SyncReport {
        let Some(link) = self.link() else {
            return SyncReport::unpaired();
        };
        let last = lock(&self.last).clone();
        let phase = if self.syncing.load(Ordering::SeqCst) {
            SyncPhase::Syncing
        } else {
            last.as_ref().map_or(SyncPhase::Starting, |l| l.phase)
        };
        SyncReport {
            phase,
            message: last.as_ref().and_then(|l| l.error.clone()),
            last_ok_ms: last.and_then(|l| l.ok_ms).map(|ms| ms.max(0) as u64),
            server: Some(link.server_url),
        }
    }

    /// Ask the foreground loop for a round once nothing more has asked for `settle`.
    fn nudge(self: &Arc<Self>, settle: Duration) {
        if self.link().is_none() {
            return;
        }
        let mut kicks = lock(&self.kicks);
        if let Some(tx) = kicks.as_ref()
            && tx.send(settle).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel::<Duration>();
        let core = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("vault-sync".into())
            .spawn(move || {
                while let Ok(mut settle) = rx.recv() {
                    while let Ok(next) = rx.recv_timeout(settle) {
                        settle = next;
                    }
                    let Some(core) = core.upgrade() else { return };
                    if let Err(e) = core.sync_now() {
                        log::warn!("sync failed: {e}");
                    }
                }
            })
            .expect("starting the sync thread");
        let _ = tx.send(settle);
        *kicks = Some(tx);
    }

    /// The app came to the front (`true`) or left it. In front, a paired core follows the home
    /// server's live events and pulls what changes there; out of it, it stops within a heartbeat
    /// and the server's `note.changed` push takes over.
    pub fn set_foreground(self: &Arc<Self>, on: bool) {
        let mut live = lock(&self.live);
        live.foreground = on;
        if !on || live.running || self.link().is_none() {
            return;
        }
        live.running = true;
        let core = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("vault-live".into())
            .spawn(move || follow_events(core))
            .expect("starting the live events thread");
    }

    /// Store what another app shared: keep its files as assets, then queue a `share_received`
    /// event with them as its attachments and run it now, so the device file's handler decides
    /// the note, by default a fleeting note in the inbox. Then ask for a round. Works offline:
    /// what the handler writes waits in the sync queue.
    ///
    /// The share is never silently lost. When no device runtime is open, or the device file
    /// declares no `share_received` handler, the fleeting note is written directly instead
    /// ([`share::write_note`]). A handler that fails leaves the event queued for the next pass,
    /// as any device event; its outcome says so. One share runs one of the two, never both.
    pub fn share(self: &Arc<Self>, shared: &Shared) -> anyhow::Result<Outcome> {
        if shared.is_empty() {
            anyhow::bail!("nothing was shared");
        }
        let writer = self.server.writer();
        let attachments = share::store(writer, shared)?;
        let payload = share::payload(shared, &attachments);
        let runtime = match self.runtime() {
            Ok(runtime) if runtime.handles("share_received")? => Some(runtime),
            _ => None,
        };
        let outcome = match runtime {
            Some(runtime) => {
                let id = runtime.enqueue("share_received", payload, now_ms())?;
                let e = runtime
                    .run_event(id)?
                    .ok_or_else(|| anyhow::anyhow!("share event {id} left the queue"))?;
                Outcome {
                    event: Some(id),
                    status: e.status.as_str().to_owned(),
                    message: e.message,
                    notes: e.notes,
                }
            }
            None => Outcome {
                event: None,
                status: "done".to_owned(),
                message: None,
                notes: vec![share::write_note(writer, shared, &payload)?],
            },
        };
        share::remove_copies(shared);
        self.kick();
        Ok(outcome)
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

    /// The bytes of asset `id`, fetched from the home server first when the device does not hold
    /// them: a person's photo the phone writes to its contact. `Ok(None)` when neither has it.
    /// It may block on the network.
    pub fn asset_bytes(&self, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let writer = self.server.writer();
        if writer.asset_info(id).is_err() && !self.fetch_asset(id)? {
            return Ok(None);
        }
        Ok(Some(writer.asset(id)?.1))
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
    /// the assets its fields and blocks name: `[{id, type, fields, blocks, assets: [{id,
    /// filename, size}]}]`.
    pub fn queued(&self) -> anyhow::Result<Value> {
        let writer = self.server.writer();
        let mut out = Vec::new();
        for p in self.server.queue().pending()? {
            let Ok(note) = writer.read(&p.note_id) else {
                out.push(json!({ "id": p.note_id, "deleted": true }));
                continue;
            };
            let doc = writer::doc_to_json(&note.doc);
            let mut ids: Vec<&str> = Vec::new();
            for v in doc["fields"]
                .as_object()
                .into_iter()
                .flat_map(|f| f.values())
            {
                match v {
                    Value::String(s) => ids.push(s.as_str()),
                    Value::Array(a) => ids.extend(a.iter().filter_map(Value::as_str)),
                    _ => {}
                }
            }
            block_assets(&doc["blocks"], &mut ids);
            let mut assets = Vec::new();
            for asset in ids.into_iter().filter_map(|i| writer.asset_info(i).ok()) {
                assets.push(json!({
                    "id": asset.id,
                    "filename": asset.filename,
                    "size": asset.size,
                }));
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
            "live": lock(&self.live).connected,
        }))
    }
}

/// The core as its embedded server's sync source. Weak, since the core holds the server.
struct Source(Weak<Core>);

impl SyncSource for Source {
    fn report(&self) -> SyncReport {
        self.0
            .upgrade()
            .map_or_else(SyncReport::unpaired, |core| core.sync_report())
    }

    fn sync_now(&self) {
        if let Some(core) = self.0.upgrade() {
            core.sync_soon();
        }
    }
}

/// Follow the home server's events while the app is in front and paired, as
/// [`Core::set_foreground`] says.
fn follow_events(core: Weak<Core>) {
    let mut backoff = MIN_BACKOFF;
    loop {
        let Some(core) = core.upgrade() else { return };
        let link = {
            let mut live = lock(&core.live);
            match core.link() {
                Some(link) if live.foreground => link,
                _ => {
                    live.running = false;
                    return;
                }
            }
        };
        let keep = || lock(&core.live).foreground && core.link().as_ref() == Some(&link);
        let mut opened = false;
        let ended = live::follow(&link.remote(), live::SILENCE, &keep, &mut |seen| {
            let news = match seen {
                Seen::Opened => {
                    opened = true;
                    lock(&core.live).connected = true;
                    true
                }
                Seen::NoteChanged(change) => live::is_news(core.server.queue(), &change),
                // The round takes the new settings first, then catches up the contacts.
                Seen::DeviceSettings(device) => device == link.device_id,
            };
            if news {
                core.nudge(REMOTE_SETTLE);
            }
        });
        lock(&core.live).connected = false;
        if opened {
            backoff = MIN_BACKOFF;
        }
        match ended {
            Ok(()) => continue,
            Err(sync::Error::Server { status, .. }) if status == 401 || status == 403 => {
                // A revoked device: the next time the app comes to the front tries again.
                let mut live = lock(&core.live);
                live.running = false;
                return;
            }
            Err(e) => log::warn!("the live event stream ended: {e}"),
        }
        // Wait before reconnecting, but stop at once when the app leaves the front.
        let until = Instant::now() + backoff;
        while Instant::now() < until && keep() {
            std::thread::sleep(Duration::from_millis(250));
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// The asset ids `blocks` name, at any depth, in order: each `asset` and `image` block's `asset`.
fn block_assets<'a>(blocks: &'a Value, out: &mut Vec<&'a str>) {
    for b in blocks.as_array().into_iter().flatten() {
        if let Some(id) = b["fields"]["asset"].as_str()
            && (b["kind"] == "asset" || b["kind"] == "image")
        {
            out.push(id);
        }
        block_assets(&b["blocks"], out);
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
