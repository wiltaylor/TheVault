//! The device side of sync: the queue of local writes, the change-feed pull, the three-way merge
//! and conflict copies, and the version handshake (D19, D20, D27, D28, D29). Both apps use it.
//!
//! A device keeps a full copy of the notes in a local vault, served in-process by
//! `crates/embedded` (D18). The home server holds the master copy. [`Engine::sync`] runs one
//! round:
//!
//! 1. **Handshake.** `GET /api/sync/info`. When the server's protocol or format version is newer
//!    than this build supports, the round stops with [`Error::ServerNewer`] and the queue stays as
//!    it is, until the app is updated (D27). The first handshake records the server's vault id in
//!    the local vault; a server whose vault id differs from the recorded one is another vault, and
//!    the round stops with [`Error::OtherVault`] before it pushes anything, for the app to start
//!    a fresh local vault (ticket 143).
//! 2. **Bases.** `PUT /api/devices/{id}/bases` with the base version of every queued change, so
//!    the server keeps those versions through pruning and browser-save merging (D29).
//! 3. **Assets the device stored** go up first with `PUT /api/assets/{id}`, under their own ids,
//!    so the notes that name them validate on the server. Each streams from its file, and fails
//!    only when no bytes move for a while ([`Timeouts::stall`]), however long it takes. An asset
//!    that fails does not fail the round: it is recorded ([`AssetFailure`]) and tried again by
//!    later rounds, waiting longer after each failure, and the queued notes that name it are held
//!    back ([`Held`]) since the server refuses a note naming an asset it does not have. The other
//!    notes go on. Only a server that cannot be reached at all, or refuses the token, fails the
//!    round here (ticket 141).
//! 4. **Push.** Each queued note, oldest first, goes through the normal notes API with its base in
//!    `If-Match`: `PUT /api/notes/{id}`, a create under the note's own id (`If-None-Match: *`) for a
//!    note the server never had, or `DELETE /api/notes/{id}` for a note the device deleted. On 409
//!    the device merges base, device and server ([`merge`]). A clean merge is written back at the
//!    server's version. A clash keeps the server's version, saves the device's as a conflict copy
//!    titled `<title> (conflict, <device name>)` whose `conflict_of` field names the original,
//!    and adds a notification that opens the copy. Either way the device takes the server's
//!    result as its own. A change whose base the server never had, and that is not in the
//!    server's trash either, is pushed as a create under the note's id with what the device
//!    holds. A note the server still refuses (a 4xx other than the 409 merge) is held back with
//!    the reason ([`Held`]) and the rest of the queue goes on: the round completes and the
//!    report names it (ticket 143).
//!    Then the notifications made on the device, such as by a device script, move to the server
//!    with `POST /api/notifications`: the app's bell shows the server's.
//! 5. **Pull.** `GET /api/assets` first, so the round knows the files it will fetch and their
//!    sizes, then `GET /api/sync/changes` from the stored cursor, a page at a time. Definitions
//!    and `devices/*.wcl` are written first, then the notes, exactly as the server holds them, and
//!    indexed. A note with a change still queued is left for the next push to merge.
//! 6. **Assets** follow the [`AssetPolicy`]: the desktop keeps every asset; Android fetches
//!    assets when a note needs them, prefetches those of pinned and recently opened notes, and
//!    evicts the least recently used beyond its cache size (D19).
//!
//! While a round runs it keeps a [`SyncProgress`] up to date: its stage, the feed entries and
//! asset files done out of those to do, the asset bytes as they arrive, and about how long is
//! left ([`progress`], ticket 144).
//! 7. **Bases** again, now for what is still queued.
//!
//! Before the round an app asks the version gate ([`gate`]) whether to run it at all: with a
//! server newer than the app and a build of the app published, the app updates itself first.
//!
//! Local writes reach the queue through [`Queue::record`], which `crates/embedded` calls from its
//! writer hooks after every committed write. What the pull writes goes through
//! [`writer::Writer::apply_remote`], which runs no hooks, so it is never queued back. Definitions
//! (types, templates, taxonomies, scripts, tasks) are never queued: they are notes of no kind, so
//! a device edits them only online, through the server's API (D20).
//!
//! The engine is blocking: it makes HTTP requests with a blocking client and writes through the
//! blocking writer. Call it from a thread of its own, never from inside an async runtime.

mod client;
pub mod gate;
pub mod live;
pub mod merge;
pub mod progress;
mod queue;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use schema::{Doc, Value};
use serde_json::{Value as Json, json};
use writer::{
    CONFLICT_OF, Input, Remote as RemoteChange, Writer, doc_to_json, is_archived, note_path,
};

use client::Home;
pub use client::Timeouts;
pub use merge::{Merged, merge};
pub use progress::{Progress, Stage, SyncProgress};
pub use queue::{AssetFailure, Held, KnownAsset, Pending, Queue, Synced};

/// How many feed entries one pull request asks for.
const PAGE: usize = 500;

/// How many times one note is written back after merging before the round gives up on it: the
/// server can change again between the merge and the write.
const MERGE_ATTEMPTS: usize = 4;

/// The home server of a paired device, and who the device is (D2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// The server's base URL, such as `https://vault.example.net`.
    pub url: String,
    /// The device token that pairing returned.
    pub token: String,
    pub device_id: String,
    /// The device's name, which titles its conflict copies.
    pub device_name: String,
}

/// The newest protocol and vault format this build can sync with (D27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Supported {
    pub protocol: u32,
    pub format_version: u32,
}

impl Default for Supported {
    fn default() -> Self {
        Supported {
            protocol: vocab::SYNC_PROTOCOL,
            format_version: vocab::FORMAT_VERSION,
        }
    }
}

/// Which assets a device keeps (D19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPolicy {
    /// Every asset the server holds: the desktop app.
    All,
    /// Assets fetched when needed: the Android app. Each round prefetches the assets of pinned
    /// notes and of the `recent` notes opened most recently, then evicts the least recently used
    /// assets the server also holds until the cache is at most `cache_bytes`. Assets the
    /// prefetch wants are never evicted, nor are assets the server does not have yet.
    OnDemand { cache_bytes: u64, recent: usize },
}

impl AssetPolicy {
    /// The desktop's policy.
    pub fn desktop() -> AssetPolicy {
        AssetPolicy::All
    }

    /// Android's default policy: a 512 MiB cache and the 50 most recently opened notes. The app's
    /// setting replaces the size.
    pub fn android() -> AssetPolicy {
        AssetPolicy::OnDemand {
            cache_bytes: 512 * 1024 * 1024,
            recent: 50,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server is newer than this build: the app must be updated before it syncs (D27).
    #[error(
        "the server uses sync protocol {server_protocol} and vault format {server_format}, newer \
         than this app supports (protocol {protocol}, format {format}): update the app; your \
         changes wait until then"
    )]
    ServerNewer {
        server_protocol: u32,
        server_format: u32,
        protocol: u32,
        format: u32,
    },
    /// The version gate paused sync: the app updates itself to `version` first ([`gate`], D27).
    #[error("{message}")]
    Updating { version: String, message: String },
    /// The server could not be reached, or the connection failed part way.
    #[error("cannot reach the server: {0}")]
    Http(String),
    /// An asset upload or download broke or stalled part way.
    #[error("{0}")]
    Transfer(String),
    /// The server refused a request.
    #[error("{what}: the server answered {status}: {body}")]
    Server {
        what: String,
        status: u16,
        body: String,
    },
    /// The server answered something the protocol does not allow.
    #[error("{0}")]
    Protocol(String),
    /// The server holds another vault than the one the local vault is a copy of: the app starts
    /// a fresh local vault before it syncs (ticket 143).
    #[error(
        "the server holds vault {server}, but this device's local vault is a copy of vault \
         {local}"
    )]
    OtherVault { local: String, server: String },
    #[error(transparent)]
    Writer(#[from] writer::Error),
    #[error("the sync tables: {0}")]
    Queue(#[from] rusqlite::Error),
    #[error(transparent)]
    State(#[from] state::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A clash that became a conflict copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub note_id: String,
    pub copy_id: String,
    /// Where the two changes overlapped.
    pub reason: String,
}

/// What one round did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Notes whose queued change reached the server, merged or not.
    pub pushed: Vec<String>,
    /// Notes whose change merged with the server's.
    pub merged: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// Feed entries the pull read.
    pub pulled: usize,
    pub assets_fetched: usize,
    pub assets_evicted: usize,
    /// Notifications made on the device that moved to the server.
    pub notifications_pushed: usize,
    /// Assets the device stored that reached the server.
    pub assets_sent: usize,
    /// Assets that failed to reach the server in this round.
    pub assets_failed: usize,
    /// Every asset still waiting to reach the server after the round, because it failed in this
    /// round or an earlier one.
    pub stuck: Vec<AssetFailure>,
    /// The queued notes held back, because they name a stuck asset or the server refused them.
    pub held: Vec<Held>,
}

/// Whether a failed push of one note is that note's own problem, which holds it back while the
/// rest of the queue goes on: the server refused the note (a 4xx other than a refused token or
/// a request to slow down) or answered it with something the protocol does not allow. A server
/// that cannot be reached, refuses the token or fails ends the round.
fn held_for(e: &Error) -> bool {
    match e {
        Error::Server { status, .. } => {
            (400..500).contains(status) && !matches!(status, 401 | 403 | 408 | 429)
        }
        Error::Protocol(_) => true,
        _ => false,
    }
}

/// `size` bytes in words, such as `800 KB` or `72.0 MB`.
pub fn size_text(size: u64) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", size.div_ceil(1024))
    }
}

impl Report {
    /// What went wrong in a round that otherwise finished: the notes the server refused, the
    /// assets that cannot reach the server and the notes waiting for them. `None` when nothing
    /// did.
    pub fn problem(&self) -> Option<String> {
        let refused: Vec<&Held> = self.held.iter().filter(|h| h.asset_id.is_none()).collect();
        let notes = refused.first().map(|first| {
            let n = refused.len();
            let mut out = format!(
                "{n} {} could not be sent to the server: note {}: {}",
                if n == 1 { "note" } else { "notes" },
                first.note_id,
                first.reason,
            );
            if n > 1 {
                out.push_str(&format!(" (and {} more)", n - 1));
            }
            out.push_str(". The others synced; it is tried again on later rounds.");
            out
        });
        let assets = self.asset_problem();
        match (notes, assets) {
            (Some(n), Some(a)) => Some(format!("{n} {a}")),
            (n, a) => n.or(a),
        }
    }

    fn asset_problem(&self) -> Option<String> {
        let first = self.stuck.first()?;
        let n = self.stuck.len();
        let mut out = format!(
            "{n} {} could not be sent to the server: {} ({}): {}",
            if n == 1 { "asset" } else { "assets" },
            first.filename,
            size_text(first.size),
            first.error,
        );
        if n > 1 {
            out.push_str(&format!(" (and {} more)", n - 1));
        }
        out.push('.');
        let held = self.held.iter().filter(|h| h.asset_id.is_some()).count();
        if held > 0 {
            out.push_str(&format!(
                " {held} {} for {}; the others synced.",
                if held == 1 {
                    "note waits"
                } else {
                    "notes wait"
                },
                if n == 1 { "it" } else { "them" },
            ));
        }
        out.push_str(" It is tried again on later rounds.");
        Some(out)
    }
}

/// The sync engine of one device.
pub struct Engine {
    writer: Arc<Writer>,
    queue: Arc<Queue>,
    home: Home,
    supported: Supported,
    assets: AssetPolicy,
    progress: SyncProgress,
}

/// How long after its `attempts`-th failure in a row an asset waits to be tried again: `first`,
/// doubling with each failure, at most an hour.
fn retry_wait(first: std::time::Duration, attempts: u32) -> std::time::Duration {
    first
        .saturating_mul(1u32 << attempts.saturating_sub(1).min(16))
        .min(std::time::Duration::from_secs(60 * 60))
}

/// The asset ids `doc` may name: every ULID string in its fields and blocks, at any depth.
fn named_ids(doc: &Doc) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Str(s) if schema::is_ulid(s) => out.push(s.clone()),
            Value::List(items) => items.iter().for_each(|i| walk(i, out)),
            Value::Record(items) => items.iter().for_each(|(_, i)| walk(i, out)),
            _ => {}
        }
    }
    fn of(d: &Doc, out: &mut Vec<String>) {
        for (_, v) in &d.fields {
            walk(v, out);
        }
        for b in &d.blocks {
            of(&b.body, out);
        }
    }
    let mut out = Vec::new();
    of(doc, &mut out);
    out
}

/// A note as the server answered with it.
struct ServerNote {
    type_name: String,
    version: String,
    wcl: String,
}

impl ServerNote {
    fn from_json(body: &Json, id: &str) -> Result<ServerNote> {
        let s = |k: &str| {
            body.get(k)
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| Error::Protocol(format!("note {id} came without `{k}`: {body}")))
        };
        Ok(ServerNote {
            type_name: s("type")?,
            version: s("version")?,
            wcl: s("wcl")?,
        })
    }
}

fn parse(id: &str, type_name: &str, wcl: &str) -> Result<Doc> {
    Ok(Input::Wcl(wcl.to_owned()).parse(&note_path(type_name, id))?)
}

fn title_of(doc: &Doc) -> String {
    doc.field("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled")
        .to_owned()
}

impl Engine {
    /// The engine of the local vault behind `writer`, whose local writes `queue` records, syncing
    /// with `remote`.
    pub fn new(
        writer: Arc<Writer>,
        queue: Arc<Queue>,
        remote: Remote,
        assets: AssetPolicy,
    ) -> Result<Engine> {
        Ok(Engine {
            writer,
            queue,
            home: Home::new(remote, Timeouts::default())?,
            supported: Supported::default(),
            assets,
            progress: SyncProgress::new(),
        })
    }

    /// Report each round's progress to `progress`, which the app reads, rather than to one of
    /// the engine's own.
    pub fn with_progress(mut self, progress: SyncProgress) -> Engine {
        self.progress = progress;
        self
    }

    /// Where the current or the last round got to.
    pub fn progress(&self) -> &SyncProgress {
        &self.progress
    }

    /// Wait on the network as `timeouts` says rather than [`Timeouts::default`].
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Result<Engine> {
        let remote = self.home.remote().clone();
        self.home = Home::new(remote, timeouts)?;
        Ok(self)
    }

    /// Sync as a build that supports at most `supported`, such as an older app.
    pub fn with_supported(mut self, supported: Supported) -> Engine {
        self.supported = supported;
        self
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Step 1: check the server is not newer than this build (D27).
    pub fn handshake(&self) -> Result<()> {
        self.handshake_gated(&mut |_| Ok(()))
    }

    /// The handshake, asking `gate` first with the server's own version (`server_version`, empty
    /// when the server names none). The version gate ([`gate`]) answers
    /// [`Error::Updating`] to pause the round before it pushes or pulls anything.
    fn handshake_gated(&self, gate: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
        let info = self.home.info()?.ok("reading the server's sync info")?;
        gate(
            info.get("server_version")
                .and_then(Json::as_str)
                .unwrap_or_default(),
        )?;
        let num = |k: &str| {
            info.get(k)
                .and_then(Json::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| Error::Protocol(format!("the sync info has no `{k}`: {info}")))
        };
        let (server_protocol, server_format) = (num("protocol")?, num("format_version")?);
        if server_protocol > self.supported.protocol
            || server_format > self.supported.format_version
        {
            return Err(Error::ServerNewer {
                server_protocol,
                server_format,
                protocol: self.supported.protocol,
                format: self.supported.format_version,
            });
        }
        // An older server names no vault: nothing to check.
        if let Some(server) = info
            .get("vault_id")
            .and_then(Json::as_str)
            .filter(|v| !v.is_empty())
        {
            match self.queue.server_vault()? {
                Some(local) if local != server => {
                    return Err(Error::OtherVault {
                        local,
                        server: server.to_owned(),
                    });
                }
                Some(_) => {}
                None => self.queue.set_server_vault(server)?,
            }
        }
        Ok(())
    }

    /// The id of the vault the home server holds, from `GET /api/sync/info`; `None` from a server
    /// too old to say.
    pub fn server_vault(&self) -> Result<Option<String>> {
        let info = self.home.info()?.ok("reading the server's sync info")?;
        Ok(info
            .get("vault_id")
            .and_then(Json::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned))
    }

    /// The app builds the server publishes, `GET /api/clients`.
    pub fn clients(&self) -> Result<Json> {
        self.home
            .get("/api/clients")?
            .ok("reading the server's app builds")
    }

    /// Run one round: handshake, push, pull and assets. See the crate documentation.
    pub fn sync(&self) -> Result<Report> {
        self.sync_gated(&mut |_| Ok(()))
    }

    /// [`Engine::sync`], with the version gate: `gate` hears the server's version first and may
    /// stop the round, with [`Error::Updating`], before it pushes or pulls anything (D27).
    pub fn sync_gated(&self, gate: &mut dyn FnMut(&str) -> Result<()>) -> Result<Report> {
        self.progress.begin();
        let result = self.handshake_gated(gate).and_then(|()| {
            self.progress.stage(Stage::Pushing);
            self.round()
        });
        self.progress.finish(result.is_ok());
        result
    }

    /// A round after its handshake.
    fn round(&self) -> Result<Report> {
        let mut report = Report::default();
        self.report_bases()?;
        let missing = self.push_assets(&mut report)?;
        let mut held = Vec::new();
        for p in self.queue.pending()? {
            if let Some(h) = self.holds(&p, &missing)? {
                held.push(h);
                continue;
            }
            match self.push_one(&p, &mut report) {
                Ok(()) => {}
                Err(e) if held_for(&e) => {
                    held.push(Held {
                        note_id: p.note_id.clone(),
                        asset_id: None,
                        reason: e.to_string(),
                    });
                }
                Err(e) => return Err(e),
            }
        }
        self.queue.set_held(&held)?;
        report.held = held;
        self.push_notifications(&mut report)?;
        self.pull(&mut report)?;
        self.sync_assets(&mut report)?;
        self.report_bases()?;
        report.stuck = self.queue.asset_failures()?;
        Ok(report)
    }

    /// Move the notifications made on the device, such as by a device script's `vault::notify`,
    /// to the home server, whose notifications the app's bell shows. Each is added there, then
    /// removed here. The notes they open were pushed before them.
    fn push_notifications(&self, report: &mut Report) -> Result<()> {
        let state = self.writer.state();
        loop {
            let page = state.notifications(false, 100, None)?;
            if page.items.is_empty() {
                return Ok(());
            }
            for n in page.items {
                let mut body = json!({ "title": n.title });
                if let Some(b) = &n.body {
                    body["body"] = json!(b);
                }
                if let Some(note) = &n.note_id {
                    body["note"] = json!(note);
                }
                if let Some(url) = &n.url {
                    body["url"] = json!(url);
                }
                self.home
                    .post_json("/api/notifications", &body)?
                    .ok("moving a notification to the server")?;
                match state.delete_notification(&n.id) {
                    Ok(()) | Err(state::Error::UnknownNotification(_)) => {}
                    Err(e) => return Err(e.into()),
                }
                report.notifications_pushed += 1;
            }
        }
    }

    /// Tell the server which base versions the queued changes depend on (D29). A base the server
    /// no longer has is left out, since the device holds its text anyway.
    pub fn report_bases(&self) -> Result<()> {
        let mut bases: BTreeMap<String, String> = self
            .queue
            .pending()?
            .into_iter()
            .filter(|p| !p.base_deleted)
            .filter_map(|p| Some((p.note_id, p.base_version?)))
            .collect();
        let path = format!("/api/devices/{}/bases", self.home.remote().device_id);
        let answer = self.home.put_json(&path, &json!({ "bases": bases }))?;
        if answer.status.as_u16() != 400 || answer.str("error") != Some("unknown_version") {
            answer.ok("recording the device's bases")?;
            return Ok(());
        }
        let mut kept = BTreeMap::new();
        for (id, v) in std::mem::take(&mut bases) {
            if self
                .home
                .get(&format!("/api/notes/{id}/history/{v}"))?
                .status
                .is_success()
            {
                kept.insert(id, v);
            }
        }
        self.home
            .put_json(&path, &json!({ "bases": kept }))?
            .ok("recording the device's bases")?;
        Ok(())
    }

    /// Step 4 for one queued note.
    fn push_one(&self, p: &Pending, report: &mut Report) -> Result<()> {
        let local = match self.writer.read(&p.note_id) {
            Ok(note) => Some(note),
            Err(writer::Error::NotFound(_)) => None,
            Err(e) => return Err(e.into()),
        };
        match local {
            Some(note) => self.push_put(p, note, report),
            None => self.push_delete(p, report),
        }
    }

    /// Push a note the device deleted. A server that changed the note since keeps it: the change
    /// wins over the delete, and the device takes the note back.
    fn push_delete(&self, p: &Pending, report: &mut Report) -> Result<()> {
        let id = &p.note_id;
        let base = match (&p.base_version, p.base_deleted) {
            (Some(base), false) => base,
            // The server never had it, or had already deleted it.
            _ => {
                self.queue.dequeue(id)?;
                return Ok(());
            }
        };
        let answer = self.home.delete_note(id, base)?;
        match answer.status.as_u16() {
            200 | 404 => {
                let version = answer.str("version").unwrap_or(base).to_owned();
                self.queue.set_synced(id, &version, None, true)?;
                self.queue.dequeue(id)?;
                report.pushed.push(id.clone());
            }
            409 => {
                let server = self.server_note(id)?;
                let lv = self.writer.apply_remote(&RemoteChange::Put {
                    id: id.clone(),
                    type_name: server.type_name,
                    wcl: server.wcl,
                })?;
                self.queue
                    .set_synced(id, &server.version, lv.as_deref(), false)?;
                self.queue.dequeue(id)?;
            }
            _ => {
                answer.ok(&format!("deleting note {id}"))?;
            }
        }
        Ok(())
    }

    fn server_note(&self, id: &str) -> Result<ServerNote> {
        let body = self
            .home
            .get(&format!("/api/notes/{id}"))?
            .ok(&format!("reading note {id}"))?;
        ServerNote::from_json(&body, id)
    }

    /// Push a note the device created or changed.
    fn push_put(&self, p: &Pending, note: writer::Note, report: &mut Report) -> Result<()> {
        let id = &p.note_id;
        let local_doc = note.doc.clone();
        let mut base_doc = match &p.base_wcl {
            Some(text) => Some(parse(id, &note.type_name, text)?),
            None => None,
        };
        let mut base_version = p.base_version.clone();
        if p.base_deleted && base_version.is_some() {
            base_version = self.restore(id)?;
        }
        let mut body = json!({ "wcl": note.wcl });
        let mut merged = false;
        for _ in 0..MERGE_ATTEMPTS {
            let answer = self
                .home
                .put_note(id, &note.type_name, &body, base_version.as_deref())?;
            match answer.status.as_u16() {
                200 | 201 => {
                    let saved = ServerNote::from_json(&answer.body, id)?;
                    let saved = self.carry_archive(id, saved, base_doc.as_ref(), &local_doc)?;
                    self.settle(id, &note.version, &saved)?;
                    report.pushed.push(id.clone());
                    if merged {
                        report.merged.push(id.clone());
                    }
                    return Ok(());
                }
                404 if base_version.is_some() => {
                    // The server deleted the note while the device changed it: the change brings
                    // it back, and merges with what it held.
                    base_version = self.restore(id)?;
                    if base_version.is_none() {
                        // Neither the note nor its trash: the server never had the base, as for
                        // a note from another vault. What the device holds goes up as a create
                        // under the note's id (ticket 143).
                        base_doc = None;
                        body = json!({ "wcl": note.wcl });
                    }
                }
                409 => {
                    let server = if answer.str("error") == Some("conflict") {
                        let get = |k: &str| answer.str(k).map(str::to_owned);
                        match (get("current_version"), get("current")) {
                            (Some(version), Some(wcl)) => ServerNote {
                                type_name: note.type_name.clone(),
                                version,
                                wcl,
                            },
                            _ => self.server_note(id)?,
                        }
                    } else {
                        // `id_taken`: the server has the note though the device never pushed
                        // it, as when a push landed and its answer was lost.
                        self.server_note(id)?
                    };
                    let server_doc = parse(id, &server.type_name, &server.wcl)?;
                    match merge(base_doc.as_ref(), &local_doc, &server_doc) {
                        Merged::Clean(doc) if doc == server_doc => {
                            self.settle(id, &note.version, &server)?;
                            report.pushed.push(id.clone());
                            report.merged.push(id.clone());
                            return Ok(());
                        }
                        Merged::Clean(doc) => {
                            body = json!({ "note": doc_to_json(&doc) });
                            base_version = Some(server.version);
                            merged = true;
                        }
                        Merged::Clash(reason) => {
                            let copy =
                                self.conflict_copy(id, &note.type_name, &local_doc, &reason)?;
                            self.settle(id, &note.version, &server)?;
                            report.conflicts.push(Conflict {
                                note_id: id.clone(),
                                copy_id: copy,
                                reason,
                            });
                            return Ok(());
                        }
                    }
                }
                _ => {
                    answer.ok(&format!("writing note {id}"))?;
                }
            }
        }
        Err(Error::Protocol(format!(
            "note {id} kept changing on the server while it was merged"
        )))
    }

    /// Bring note `id` back from the server's trash. Returns its version there, or `None` when it
    /// is not in the trash.
    fn restore(&self, id: &str) -> Result<Option<String>> {
        let answer = self
            .home
            .post_json(&format!("/api/trash/{id}/restore"), &json!({}))?;
        match answer.status.as_u16() {
            200 | 201 => Ok(answer.str("version").map(str::to_owned)),
            // Already live: its current version is in the conflict.
            409 => Ok(answer.str("current_version").map(str::to_owned)),
            404 => Ok(None),
            _ => {
                answer.ok(&format!("restoring note {id}"))?;
                Ok(None)
            }
        }
    }

    /// A write keeps the archived state the server's note has, so an archive or unarchive the
    /// device made is sent on its own afterwards.
    fn carry_archive(
        &self,
        id: &str,
        saved: ServerNote,
        base: Option<&Doc>,
        local: &Doc,
    ) -> Result<ServerNote> {
        let wanted = is_archived(local);
        let was = base.is_some_and(is_archived);
        let now = is_archived(&parse(id, &saved.type_name, &saved.wcl)?);
        if wanted == was || wanted == now {
            return Ok(saved);
        }
        let action = if wanted { "archive" } else { "unarchive" };
        let body = self
            .home
            .post_at(&format!("/api/notes/{id}/{action}"), &saved.version)?
            .ok(&format!("{action} note {id}"))?;
        ServerNote::from_json(&body, id)
    }

    /// Take the server's result for note `id` as the local copy, unless the note changed locally
    /// while it was pushed (from `pushed`, its local version then): that change stays queued,
    /// now based on the server's result.
    fn settle(&self, id: &str, pushed: &str, server: &ServerNote) -> Result<()> {
        if self.writer.current_version(id)?.as_deref() != Some(pushed) {
            self.queue.rebase(id, &server.version, &server.wcl)?;
            self.queue.set_synced(id, &server.version, None, false)?;
            return Ok(());
        }
        let lv = self.writer.apply_remote(&RemoteChange::Put {
            id: id.to_owned(),
            type_name: server.type_name.clone(),
            wcl: server.wcl.clone(),
        })?;
        self.queue
            .set_synced(id, &server.version, lv.as_deref(), false)?;
        self.queue.dequeue(id)?;
        Ok(())
    }

    /// Save the device's side of a clash as a new note on the server, titled `<title> (conflict,
    /// <device name>)`, with `conflict_of` naming the original, and add a notification that opens
    /// it (D20). Returns the copy's id. The copy reaches the device with the next pull.
    fn conflict_copy(
        &self,
        id: &str,
        type_name: &str,
        local: &Doc,
        reason: &str,
    ) -> Result<String> {
        let device = &self.home.remote().device_name;
        let title = title_of(local);
        let computed: HashSet<String> = self
            .writer
            .schema()
            .type_def(type_name)
            .map(|t| {
                t.fields
                    .iter()
                    .filter(|f| f.computed)
                    .map(|f| f.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut copy = local.clone();
        copy.fields.retain(|(n, _)| {
            !matches!(n.as_str(), "title" | "created" | "updated" | "archived")
                && n != CONFLICT_OF
                && !computed.contains(n)
        });
        copy.fields.insert(
            0,
            (
                "title".to_owned(),
                Value::Str(format!("{title} (conflict, {device})")),
            ),
        );
        copy.fields
            .push((CONFLICT_OF.to_owned(), Value::Str(id.to_owned())));
        copy.blocks
            .retain(|b| !b.label().is_some_and(|l| computed.contains(l)));
        let made = self
            .home
            .post_json(
                &format!("/api/notes?type={type_name}"),
                &json!({ "note": doc_to_json(&copy) }),
            )?
            .ok(&format!("saving the conflict copy of note {id}"))?;
        let copy_id = made
            .get("id")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                Error::Protocol(format!("the conflict copy came without an id: {made}"))
            })?
            .to_owned();
        self.home
            .post_json(
                "/api/notifications",
                &json!({
                    "title": format!("Sync conflict in \"{title}\""),
                    "body": format!(
                        "{device} and the server both changed this note ({reason}). The server's \
                         version was kept and {device}'s is saved as a conflict copy."
                    ),
                    "note": copy_id,
                }),
            )?
            .ok("adding the conflict notification")?;
        Ok(copy_id)
    }

    /// Step 5: learn the server's assets, then apply the change feed from the stored cursor.
    fn pull(&self, report: &mut Report) -> Result<()> {
        self.progress.stage(Stage::Pulling);
        self.learn_assets()?;
        if self.assets == AssetPolicy::All {
            self.plan_fetch(&self.missing_assets()?);
        }
        loop {
            let since = self.queue.cursor()?;
            let answer = self
                .home
                .get(&format!("/api/sync/changes?since={since}&limit={PAGE}"))?;
            let read = answer.bytes;
            let page = answer.ok("reading the change feed")?;
            let changes = page
                .get("changes")
                .and_then(Json::as_array)
                .ok_or_else(|| Error::Protocol(format!("the feed page has no changes: {page}")))?;
            let page_next = page.get("next").and_then(Json::as_i64).unwrap_or(since);
            // An older server says only `latest`, which overstates what remains.
            let remaining = page
                .get("remaining")
                .and_then(Json::as_u64)
                .unwrap_or_else(|| {
                    let latest = page.get("latest").and_then(Json::as_i64).unwrap_or(0);
                    latest.saturating_sub(page_next).max(0) as u64
                });
            self.progress
                .feed_page(changes.len() as u64, read, remaining);
            let files: Vec<(String, Option<String>)> = changes
                .iter()
                .filter(|c| c.get("kind").and_then(Json::as_str) != Some("note"))
                .filter_map(|c| {
                    let path = c.get("path")?.as_str()?.to_owned();
                    let text = c.get("text").and_then(Json::as_str).map(str::to_owned);
                    Some((path, text))
                })
                .collect();
            if !files.is_empty() {
                self.writer.apply_remote_files(&files)?;
            }
            for c in changes
                .iter()
                .filter(|c| c.get("kind").and_then(Json::as_str) == Some("note"))
            {
                self.apply_pulled(c)?;
            }
            self.progress.notes_applied(changes.len() as u64);
            report.pulled += changes.len();
            let next = page.get("next").and_then(Json::as_i64).unwrap_or(since);
            let latest = page.get("latest").and_then(Json::as_i64).unwrap_or(next);
            self.queue.set_cursor(next)?;
            if changes.is_empty() || next >= latest {
                return Ok(());
            }
        }
    }

    /// Apply one note entry of the feed.
    fn apply_pulled(&self, c: &Json) -> Result<()> {
        let s = |k: &str| c.get(k).and_then(Json::as_str);
        let id = s("id").ok_or_else(|| Error::Protocol(format!("a feed note has no id: {c}")))?;
        let deleted = c.get("deleted").and_then(Json::as_bool).unwrap_or(false);
        if self.queue.pending_for(id)?.is_some() {
            // The next push merges the server's change with the device's.
            return Ok(());
        }
        let version = s("version").unwrap_or_default();
        if let Some(known) = self.queue.synced(id)?
            && known.server_version == version
            && known.deleted == deleted
        {
            return Ok(());
        }
        if deleted {
            self.writer
                .apply_remote(&RemoteChange::Delete { id: id.to_owned() })?;
            self.queue.set_synced(id, version, None, true)?;
            return Ok(());
        }
        let (Some(type_name), Some(wcl)) = (s("type"), s("wcl")) else {
            return Err(Error::Protocol(format!(
                "a feed note has no type or wcl: {c}"
            )));
        };
        let lv = self.writer.apply_remote(&RemoteChange::Put {
            id: id.to_owned(),
            type_name: type_name.to_owned(),
            wcl: wcl.to_owned(),
        })?;
        self.queue.set_synced(id, version, lv.as_deref(), false)?;
        Ok(())
    }

    /// Whether queued note `p` must wait, because it names an asset in `missing` that has not
    /// reached the server, which would refuse the note for it.
    fn holds(&self, p: &Pending, missing: &BTreeMap<String, AssetFailure>) -> Result<Option<Held>> {
        if missing.is_empty() {
            return Ok(None);
        }
        let note = match self.writer.read(&p.note_id) {
            Ok(note) => note,
            Err(writer::Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(named_ids(&note.doc).into_iter().find_map(|id| {
            let f = missing.get(&id)?;
            Some(Held {
                note_id: p.note_id.clone(),
                asset_id: Some(id),
                reason: format!(
                    "waits for asset {} ({}), which has not reached the server: {}",
                    f.filename,
                    size_text(f.size),
                    f.error
                ),
            })
        }))
    }

    /// Step 3: send the assets the device stored and the server does not have. An asset that
    /// fails is recorded and left for a later round, after a wait that doubles with each failure
    /// in a row. Returns the assets still not on the server, failed now or waiting to be tried
    /// again. The round fails only when the server cannot be reached or refuses the token.
    fn push_assets(&self, report: &mut Report) -> Result<BTreeMap<String, AssetFailure>> {
        let local = self.writer.assets()?;
        for f in self.queue.asset_failures()? {
            if !local.iter().any(|a| a.id == f.id) {
                self.queue.clear_asset_failure(&f.id)?;
            }
        }
        let mut missing = BTreeMap::new();
        for asset in local {
            if self
                .queue
                .asset(&asset.id)?
                .is_some_and(|known| known.on_server)
            {
                continue;
            }
            let before = self.queue.asset_failure(&asset.id)?;
            if let Some(f) = &before
                && f.next_ms > queue::now_ms()
            {
                missing.insert(asset.id.clone(), f.clone());
                continue;
            }
            let path = self.writer.vault().root().join(asset.path());
            let what = format!("sending asset {}", asset.id);
            let error = match self.home.put_asset_file(&asset.id, &asset.filename, &path) {
                Ok(answer) if answer.status.is_success() => None,
                Ok(answer) if matches!(answer.status.as_u16(), 401 | 403) => {
                    answer.ok(&what)?;
                    None
                }
                Ok(answer) => Some(
                    answer
                        .ok(&what)
                        .err()
                        .map_or_else(String::new, |e| e.to_string()),
                ),
                Err(e @ Error::Transfer(_)) => Some(e.to_string()),
                Err(e) => return Err(e),
            };
            let Some(error) = error else {
                self.queue
                    .set_asset(&asset.id, &asset.filename, asset.size, true)?;
                self.queue.clear_asset_failure(&asset.id)?;
                report.assets_sent += 1;
                continue;
            };
            let attempts = before.map_or(1, |f| f.attempts.saturating_add(1));
            let wait = retry_wait(self.home.timeouts().asset_retry, attempts);
            let now = queue::now_ms();
            let failure = AssetFailure {
                id: asset.id.clone(),
                filename: asset.filename.clone(),
                size: asset.size,
                error,
                attempts,
                last_ms: now,
                next_ms: now + wait.as_millis() as i64,
            };
            self.queue.set_asset_failure(&failure)?;
            report.assets_failed += 1;
            missing.insert(asset.id, failure);
        }
        Ok(missing)
    }

    /// What the server lists of its assets, recorded as known: the start of step 5.
    fn learn_assets(&self) -> Result<()> {
        let list = self.home.get("/api/assets")?.ok("listing the assets")?;
        for a in list
            .get("assets")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(filename), Some(size)) = (
                a.get("id").and_then(Json::as_str),
                a.get("filename").and_then(Json::as_str),
                a.get("size").and_then(Json::as_u64),
            ) else {
                continue;
            };
            self.queue.set_asset(id, filename, size, true)?;
            // The server has it after all, such as from an upload whose answer was lost.
            self.queue.clear_asset_failure(id)?;
        }
        Ok(())
    }

    /// The assets the server holds and the device does not.
    fn missing_assets(&self) -> Result<Vec<KnownAsset>> {
        Ok(self
            .queue
            .assets()?
            .into_iter()
            .filter(|a| a.on_server && !self.is_local(&a.id))
            .collect())
    }

    /// Tell the progress the round fetches `assets`.
    fn plan_fetch(&self, assets: &[KnownAsset]) {
        self.progress
            .plan_files(assets.len() as u64, assets.iter().map(|a| a.size).sum());
    }

    /// Step 6: fetch and evict under the policy, the server's assets learnt in step 5.
    fn sync_assets(&self, report: &mut Report) -> Result<()> {
        self.progress.stage(Stage::Fetching);
        match self.assets {
            AssetPolicy::All => {
                let missing = self.missing_assets()?;
                self.plan_fetch(&missing);
                for a in missing {
                    self.fetch_counted(&a.id)?;
                    report.assets_fetched += 1;
                }
            }
            AssetPolicy::OnDemand {
                cache_bytes,
                recent,
            } => {
                let mut wanted = HashSet::new();
                let mut notes = self.writer.state().pins()?;
                notes.extend(self.queue.recent(recent)?);
                for note in notes {
                    wanted.extend(self.assets_of(&note)?);
                }
                let fetch: Vec<KnownAsset> = self
                    .missing_assets()?
                    .into_iter()
                    .filter(|a| wanted.contains(&a.id))
                    .collect();
                self.plan_fetch(&fetch);
                for id in &wanted {
                    if !self.is_local(id) {
                        self.fetch_counted(id)?;
                        report.assets_fetched += 1;
                    }
                }
                report.assets_evicted += self.evict(cache_bytes, &wanted)?;
            }
        }
        Ok(())
    }

    fn is_local(&self, id: &str) -> bool {
        self.writer.asset_info(id).is_ok()
    }

    /// The ids of the assets note `id` names, of those the engine knows of.
    fn assets_of(&self, id: &str) -> Result<Vec<String>> {
        let note = match self.writer.read(id) {
            Ok(note) => note,
            Err(writer::Error::NotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for i in named_ids(&note.doc) {
            if self.queue.asset(&i)?.is_some() && !out.contains(&i) {
                out.push(i);
            }
        }
        Ok(out)
    }

    /// Evict the least recently used assets the server holds, other than `keep`, until the local
    /// ones fit in `cache_bytes`. Returns how many were evicted.
    fn evict(&self, cache_bytes: u64, keep: &HashSet<String>) -> Result<usize> {
        let mut local: Vec<KnownAsset> = self
            .queue
            .assets()?
            .into_iter()
            .filter(|a| self.is_local(&a.id))
            .collect();
        let mut total: u64 = local.iter().map(|a| a.size).sum();
        local.sort_by_key(|a| (a.used_ms, a.id.clone()));
        let mut evicted = 0;
        for a in local {
            if total <= cache_bytes {
                break;
            }
            if !a.on_server || keep.contains(&a.id) {
                continue;
            }
            self.writer.evict_asset(&a.id)?;
            total -= a.size;
            evicted += 1;
        }
        Ok(evicted)
    }

    /// Fetch asset `id` from the server into the local vault, as a note being shown needs it.
    pub fn fetch_asset(&self, id: &str) -> Result<()> {
        self.fetch_with(id, &mut |_| {})
    }

    /// [`Engine::fetch_asset`] in a round: its bytes count in the round's progress.
    fn fetch_counted(&self, id: &str) -> Result<()> {
        self.fetch_with(id, &mut |n| self.progress.bytes(n))?;
        self.progress.file_fetched();
        Ok(())
    }

    fn fetch_with(&self, id: &str, on_bytes: &mut dyn FnMut(u64)) -> Result<()> {
        let known = self
            .queue
            .asset(id)?
            .ok_or_else(|| Error::Protocol(format!("asset {id} is not one the server listed")))?;
        let incoming = self.writer.incoming_file()?;
        self.home.download_asset(id, &incoming, on_bytes)?;
        self.writer
            .put_asset_file_with_id(id, &known.filename, &incoming)?;
        self.queue.touch_asset(id)?;
        Ok(())
    }

    /// Note `id` was opened on the device: remember it for the prefetch, and fetch the assets it
    /// names that are not on the device yet.
    pub fn note_opened(&self, id: &str) -> Result<()> {
        self.queue.opened(id)?;
        for asset in self.assets_of(id)? {
            if self.is_local(&asset) {
                self.queue.touch_asset(&asset)?;
            } else {
                self.fetch_asset(&asset)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_failed_asset_waits_longer_after_each_failure_up_to_an_hour() {
        let first = Duration::from_secs(30);
        assert_eq!(retry_wait(first, 1), Duration::from_secs(30));
        assert_eq!(retry_wait(first, 2), Duration::from_secs(60));
        assert_eq!(retry_wait(first, 3), Duration::from_secs(120));
        assert_eq!(retry_wait(first, 8), Duration::from_secs(60 * 60));
        assert_eq!(retry_wait(first, 400), Duration::from_secs(60 * 60));
        assert_eq!(retry_wait(Duration::ZERO, 5), Duration::ZERO);
    }

    #[test]
    fn sizes_read_as_kb_or_mb() {
        assert_eq!(size_text(10), "1 KB");
        assert_eq!(size_text(800 * 1024), "800 KB");
        assert_eq!(size_text(72 * 1024 * 1024), "72.0 MB");
    }
}
