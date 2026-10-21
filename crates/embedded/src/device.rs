//! Device events and commands, run on the device itself (D5, D6, D17, D22, D23, D25).
//!
//! The Android app captures device events (an SMS arriving, a call ending, a contact changing,
//! something shared) and hands each to [`DeviceRuntime::enqueue`], which stores it in a durable
//! queue in the local `state.db` before anything runs. [`DeviceRuntime::run_pending`] then runs
//! each queued event through the handlers `devices/<platform>.wcl` names, as [`Kind::Device`]
//! scripts that reach the `vault` module, writing to the local vault, and the `device` module,
//! reaching the phone through the app's [`DeviceData`]. Those writes sync like any other.
//!
//! Before a handler runs, an event from anyone on the device's exclusion list, and an SMS whose
//! body matches its skip pattern, is dropped (D25). A failed run leaves the event queued for the
//! next pass, up to [`MAX_ATTEMPTS`] passes; handlers de-duplicate on `device_ref`, so a retry is
//! safe.
//!
//! A command a server script queued for the device runs through [`DeviceRuntime::run_command`]
//! only when the device file declares it and the device's settings enable it (D22).
//!
//! Contacts sync both ways (D16). A contact changing on the phone is a `contact_changed` event; a
//! linked `person` changing anywhere reaches the phone through
//! [`DeviceRuntime::queue_person_changes`], which queues the same event for its contact so that
//! the one handler settles both directions, field by field.
//!
//! A vault without `devices/<platform>.wcl` runs the built-in default device file, and a script
//! the device file names that the vault lacks runs from the built-in defaults (D15).
//!
//! An event whose handler fails on every pass is marked failed and stays in the queue until the
//! user retries it ([`DeviceRuntime::retry`]) or dismisses it ([`DeviceRuntime::dismiss`]), from
//! Settings > Devices > This device's "Capture problems", which [`DeviceRuntime::problems`] lists.
//! Each failed event is announced once on the phone ([`DeviceRuntime::new_failures`]) and reported
//! once to the home server ([`DeviceRuntime::unreported_failures`]), so a message that never
//! became a note is never silent.
//!
//! Each queued event records the notes its handlers created ([`QueuedEvent::notes`]), so a caller
//! that runs one event at once with [`DeviceRuntime::run_event`], such as the share target, can
//! tell what it made.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};
use schema::{DEVICE_EVENTS, DeviceFile};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use writer::Writer;

use script_core::{
    Archived, Author, DeviceAccess, DeviceExtensions, Host, Job, Kind, Limits, Vault, current_job,
};
pub use script_core::{DeviceData, Switches};

mod import;
pub use import::{
    IMPORT_COMMAND, IMPORT_KINDS, ImportReport, ImportRequest, KindReport, RCS_NOT_IMPORTABLE,
};

/// Passes over a failing event before it is marked failed and left alone.
pub const MAX_ATTEMPTS: i64 = 3;

/// How long finished and dropped events stay in the queue, for the record.
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS device_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    event      TEXT NOT NULL,
    data       TEXT NOT NULL,
    at_ms      INTEGER NOT NULL,
    status     TEXT NOT NULL DEFAULT 'queued',
    attempts   INTEGER NOT NULL DEFAULT 0,
    message    TEXT,
    updated_ms INTEGER NOT NULL,
    notes      TEXT NOT NULL DEFAULT '[]',
    notified   INTEGER NOT NULL DEFAULT 0,
    reported   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS device_events_status ON device_events (status, id);

CREATE TABLE IF NOT EXISTS device_runtime (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS device_people (
    id      TEXT PRIMARY KEY,
    version TEXT NOT NULL
);
";

/// The device this runtime runs on.
pub struct DeviceConfig {
    /// The device's id on the home server, from pairing.
    pub id: String,
    /// `android` or `linux`, which names the device file.
    pub platform: String,
    /// The phone's own data and actions.
    pub data: Arc<dyn DeviceData>,
}

/// What a device's exclusion list entry names (D25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExclusionKind {
    /// A phone contact, by its contact id or its display name.
    Contact,
    /// A phone number, E.164 or as the phone holds it.
    Number,
    /// A conversation, by its `thread_ref`.
    Thread,
}

/// A contact, number or thread whose events are never captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclusion {
    pub kind: ExclusionKind,
    pub value: String,
}

/// The device's settings from Settings > Devices on the home server, in the JSON form
/// `GET /api/devices` gives them (D22, D23, D25). Members it does not know are ignored, so an
/// older app keeps working with a newer server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSettings {
    /// The commands the user enabled for this device.
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub contacts_write: bool,
    #[serde(default)]
    pub sms_send: bool,
    #[serde(default)]
    pub exclusions: Vec<Exclusion>,
    /// A regular expression; an SMS whose body matches it is dropped. Empty drops nothing.
    #[serde(default = "default_skip_pattern")]
    pub skip_pattern: String,
}

fn default_skip_pattern() -> String {
    api::DEFAULT_SKIP_PATTERN.to_owned()
}

impl Default for DeviceSettings {
    fn default() -> DeviceSettings {
        DeviceSettings {
            commands: Vec::new(),
            contacts_write: false,
            sms_send: false,
            exclusions: Vec::new(),
            skip_pattern: default_skip_pattern(),
        }
    }
}

impl DeviceSettings {
    fn switches(&self) -> Switches {
        Switches {
            contacts_write: self.contacts_write,
            sms_send: self.sms_send,
        }
    }
}

/// Where a queued event stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    /// Waiting for a pass, or for another try after a failure.
    Queued,
    /// Every handler ran.
    Done,
    /// A handler failed on every one of [`MAX_ATTEMPTS`] passes.
    Failed,
    /// Dropped by the exclusion list or the skip pattern, before any handler ran.
    Skipped,
    /// Failed, then dismissed by the user from "Capture problems".
    Dismissed,
}

impl EventStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EventStatus::Queued => "queued",
            EventStatus::Done => "done",
            EventStatus::Failed => "failed",
            EventStatus::Skipped => "skipped",
            EventStatus::Dismissed => "dismissed",
        }
    }

    fn parse(s: &str) -> EventStatus {
        match s {
            "done" => EventStatus::Done,
            "failed" => EventStatus::Failed,
            "skipped" => EventStatus::Skipped,
            "dismissed" => EventStatus::Dismissed,
            _ => EventStatus::Queued,
        }
    }
}

/// One queued event and how it stands.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedEvent {
    pub id: i64,
    pub event: String,
    pub data: Json,
    pub at_ms: i64,
    pub status: EventStatus,
    pub attempts: i64,
    /// Why it was skipped, or the last failure.
    pub message: Option<String>,
    /// The ids of the notes its handlers created, over every pass, oldest first.
    pub notes: Vec<String>,
}

impl QueuedEvent {
    /// Who the event is from or about, as its payload names them: the phone contact's name, an
    /// RCS sender, a contact event's name, else the number. `None` when it names nobody, as a
    /// share does.
    pub fn who(&self) -> Option<String> {
        let d = &self.data;
        fn text(v: &Json) -> Option<&str> {
            v.as_str().map(str::trim).filter(|s| !s.is_empty())
        }
        [
            &d["contact"]["name"],
            &d["sender"],
            &d["name"],
            &d["number"],
            &d["number_raw"],
            &d["to"][0]["contact"]["name"],
            &d["to"][0]["number"],
        ]
        .into_iter()
        .find_map(text)
        .map(str::to_owned)
    }

    /// The event as "Capture problems" lists it and the home server is told of it: `{ id, event,
    /// at, status, attempts, message, who }`.
    pub fn problem_json(&self) -> Json {
        json!({
            "id": self.id,
            "event": self.event,
            "at": self.at_ms,
            "status": self.status.as_str(),
            "attempts": self.attempts,
            "message": self.message,
            "who": self.who(),
        })
    }
}

/// How a command run ended, in the form `POST /api/devices/{id}/commands/{cid}/result` takes.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandReport {
    /// The command's id.
    pub id: String,
    /// True for `done`, false for `failed`.
    pub done: bool,
    /// What the command function returned, or the failure's message.
    pub result: Json,
}

impl CommandReport {
    /// The body to report the command with.
    pub fn body(&self) -> Json {
        json!({
            "status": if self.done { "done" } else { "failed" },
            "result": self.result,
        })
    }
}

/// The device event runtime of one embedded server.
pub struct DeviceRuntime {
    writer: Arc<Writer>,
    host: Host,
    /// The vault the host's scripts reach, which records the notes each event's handlers create.
    recorder: Arc<Recorder>,
    access: Arc<DeviceAccess>,
    data: Arc<dyn DeviceData>,
    platform: String,
    settings: RwLock<DeviceSettings>,
    queue: Mutex<Connection>,
    /// Held for a whole pass, so two passes never run the same event.
    pass: Mutex<()>,
    /// Held while an import of history runs, so only one runs at a time.
    importing: Mutex<()>,
}

impl DeviceRuntime {
    /// The runtime of `config`'s device over the vault `writer` writes. `state_db` is the local
    /// `state.db`, where the queue lives.
    pub(crate) fn open(
        writer: Arc<Writer>,
        state_db: &Path,
        config: DeviceConfig,
    ) -> anyhow::Result<DeviceRuntime> {
        if !schema::DEVICE_PLATFORMS.contains(&config.platform.as_str()) {
            anyhow::bail!("{:?} is not a device platform", config.platform);
        }
        let conn = Connection::open(state_db).context("opening state.db")?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA)
            .context("creating the device event queue")?;
        // A queue made before events recorded their notes.
        if conn
            .prepare("SELECT notes FROM device_events LIMIT 0")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE device_events ADD COLUMN notes TEXT NOT NULL DEFAULT '[]'",
            )
            .context("adding the notes of device events")?;
        }
        // A queue made before failures were announced and reported.
        if conn
            .prepare("SELECT notified, reported FROM device_events LIMIT 0")
            .is_err()
        {
            conn.execute_batch(
                "ALTER TABLE device_events ADD COLUMN notified INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE device_events ADD COLUMN reported INTEGER NOT NULL DEFAULT 0;
                 UPDATE device_events SET notified = 1, reported = 1 WHERE status = 'failed';",
            )
            .context("adding the announcement of failed device events")?;
        }
        let settings = conn
            .query_row(
                "SELECT value FROM device_runtime WHERE key = 'settings'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| serde_json::from_str::<DeviceSettings>(&s).ok())
            .unwrap_or_default();
        let access = Arc::new(DeviceAccess::new(
            &config.id,
            config.data.clone(),
            settings.switches(),
        ));
        let recorder = Arc::new(Recorder {
            writer: writer.clone(),
            created: Mutex::new(HashMap::new()),
        });
        let vault: Arc<dyn Vault> = recorder.clone();
        let host = Host::with_extensions(
            vault,
            1,
            Limits::default(),
            &DeviceExtensions(access.clone()),
        );
        Ok(DeviceRuntime {
            writer,
            host,
            recorder,
            access,
            data: config.data,
            platform: config.platform,
            settings: RwLock::new(settings),
            queue: Mutex::new(conn),
            pass: Mutex::new(()),
            importing: Mutex::new(()),
        })
    }

    /// The phone this runtime reaches, to open the runtime again over another local vault.
    pub fn data(&self) -> Arc<dyn DeviceData> {
        self.data.clone()
    }

    /// Remove the phone's contact links to any vault but `vault_id` ([`DeviceData::
    /// contacts_forget_links`]), when the device may write contacts. Answers how many it removed,
    /// or `None` when the contacts write switch is off and the links stay, ignored (ticket 143).
    pub fn forget_contact_links(&self, vault_id: &str) -> anyhow::Result<Option<u64>> {
        if !self.settings().contacts_write {
            return Ok(None);
        }
        self.data
            .contacts_forget_links(vault_id)
            .map(Some)
            .map_err(anyhow::Error::msg)
    }

    /// The device's id.
    pub fn device_id(&self) -> &str {
        self.access.id()
    }

    /// The settings in force.
    pub fn settings(&self) -> DeviceSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Replace the settings, from the device's `settings` on the home server, and keep them for
    /// runs with no connection.
    pub fn set_settings(&self, settings: &Json) -> anyhow::Result<()> {
        let parsed: DeviceSettings =
            serde_json::from_value(settings.clone()).context("reading the device settings")?;
        if !parsed.skip_pattern.is_empty() {
            regex::Regex::new(&parsed.skip_pattern).context("the skip pattern")?;
        }
        self.conn().execute(
            "INSERT INTO device_runtime (key, value) VALUES ('settings', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(&parsed)?],
        )?;
        self.access.set_switches(parsed.switches());
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = parsed;
        Ok(())
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `event` with payload `data`, captured at `at_ms`. It is stored before this returns,
    /// so it survives the process ending. Returns its id.
    pub fn enqueue(&self, event: &str, data: Json, at_ms: i64) -> anyhow::Result<i64> {
        if !DEVICE_EVENTS.contains(&event) {
            anyhow::bail!(
                "{event:?} is not a device event: {}",
                DEVICE_EVENTS.join(", ")
            );
        }
        if !data.is_object() {
            anyhow::bail!("the payload of a device event is a map");
        }
        let now = self.writer.now_ms();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO device_events (event, data, at_ms, updated_ms) VALUES (?1, ?2, ?3, ?4)",
            params![event, data.to_string(), at_ms, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Queued event `id`.
    pub fn event(&self, id: i64) -> anyhow::Result<Option<QueuedEvent>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, event, data, at_ms, status, attempts, message, notes FROM device_events
                 WHERE id = ?1",
                [id],
                event_row,
            )
            .optional()?)
    }

    /// Every event in the queue, oldest first.
    pub fn events(&self) -> anyhow::Result<Vec<QueuedEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, event, data, at_ms, status, attempts, message, notes FROM device_events
             ORDER BY id",
        )?;
        let rows = stmt.query_map([], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The events "Capture problems" lists: every failed and skipped one, newest first.
    pub fn problems(&self) -> anyhow::Result<Vec<QueuedEvent>> {
        self.select("status IN ('failed', 'skipped') ORDER BY at_ms DESC, id DESC")
    }

    /// How many events failed for good and are not dismissed.
    pub fn failed_count(&self) -> anyhow::Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM device_events WHERE status = 'failed'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Give failed event `id` another [`MAX_ATTEMPTS`] passes, running the first now, and return
    /// it as it then stands. `None` when the queue holds no failed event `id`.
    pub fn retry(&self, id: i64) -> anyhow::Result<Option<QueuedEvent>> {
        let changed = self.conn().execute(
            "UPDATE device_events SET status = 'queued', attempts = 0, notified = 0, reported = 0,
             updated_ms = ?2 WHERE id = ?1 AND status = 'failed'",
            params![id, self.writer.now_ms()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.run_event(id)
    }

    /// Dismiss failed event `id`: it leaves "Capture problems" and the failed count, and the queue
    /// with the finished events. `false` when the queue holds no failed event `id`.
    pub fn dismiss(&self, id: i64) -> anyhow::Result<bool> {
        let changed = self.conn().execute(
            "UPDATE device_events SET status = 'dismissed', updated_ms = ?2
             WHERE id = ?1 AND status = 'failed'",
            params![id, self.writer.now_ms()],
        )?;
        Ok(changed > 0)
    }

    /// The events that failed for good since the last call, oldest first, each returned once, for
    /// the phone to announce.
    pub fn new_failures(&self) -> anyhow::Result<Vec<QueuedEvent>> {
        let fresh = self.select("status = 'failed' AND notified = 0 ORDER BY id")?;
        for e in &fresh {
            self.conn().execute(
                "UPDATE device_events SET notified = 1 WHERE id = ?1",
                [e.id],
            )?;
        }
        Ok(fresh)
    }

    /// The failed events the home server has not been told of yet, oldest first. Mark each told
    /// with [`DeviceRuntime::mark_reported`].
    pub fn unreported_failures(&self) -> anyhow::Result<Vec<QueuedEvent>> {
        self.select("status = 'failed' AND reported = 0 ORDER BY id")
    }

    /// The home server has been told event `id` failed.
    pub fn mark_reported(&self, id: i64) -> anyhow::Result<()> {
        self.conn()
            .execute("UPDATE device_events SET reported = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The events `filter`, a `WHERE` clause and its `ORDER BY`, picks.
    fn select(&self, filter: &str) -> anyhow::Result<Vec<QueuedEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, event, data, at_ms, status, attempts, message, notes FROM device_events
             WHERE {filter}"
        ))?;
        let rows = stmt.query_map([], event_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Run every queued event, oldest first, and return each as it then stands. Blocks while the
    /// handlers run. Finished and dropped events older than [`RETENTION_MS`] leave the queue.
    pub fn run_pending(&self) -> anyhow::Result<Vec<QueuedEvent>> {
        let _pass = self.pass.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.writer.now_ms();
        let queued: Vec<QueuedEvent> = {
            let conn = self.conn();
            conn.execute(
                "DELETE FROM device_events WHERE status IN ('done', 'skipped', 'dismissed')
                 AND updated_ms < ?1",
                [now - RETENTION_MS],
            )?;
            let mut stmt = conn.prepare(
                "SELECT id, event, data, at_ms, status, attempts, message, notes FROM device_events
                 WHERE status = 'queued' ORDER BY id",
            )?;
            let rows = stmt.query_map([], event_row)?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let settings = self.settings();
        let file = self.device_file()?;
        let mut out = Vec::with_capacity(queued.len());
        for e in queued {
            out.push(self.run_one(&settings, file.as_ref(), e)?);
        }
        Ok(out)
    }

    /// Run queued event `id` alone, now, and return it as it then stands, with the notes its
    /// handlers created. The rest of the queue waits for [`DeviceRuntime::run_pending`]. An event
    /// a pass already took is returned as that pass left it, not run again, so a pass running at
    /// the same time never runs it twice. `None` when the queue holds no event `id`.
    pub fn run_event(&self, id: i64) -> anyhow::Result<Option<QueuedEvent>> {
        let _pass = self.pass.lock().unwrap_or_else(|e| e.into_inner());
        let Some(e) = self.event(id)? else {
            return Ok(None);
        };
        if e.status != EventStatus::Queued {
            return Ok(Some(e));
        }
        let file = self.device_file()?;
        self.run_one(&self.settings(), file.as_ref(), e).map(Some)
    }

    /// Whether the device file in force declares a handler for `event`.
    pub fn handles(&self, event: &str) -> anyhow::Result<bool> {
        Ok(self
            .device_file()?
            .is_some_and(|f| f.handlers.iter().any(|h| h.name == event)))
    }

    /// One pass over queued event `e`: drop it, or run its handlers, then store how it stands.
    fn run_one(
        &self,
        settings: &DeviceSettings,
        file: Option<&DeviceFile>,
        mut e: QueuedEvent,
    ) -> anyhow::Result<QueuedEvent> {
        if let Some(why) = dropped(settings, &e.event, &e.data) {
            e.status = EventStatus::Skipped;
            e.message = Some(why);
        } else {
            e.attempts += 1;
            let ran = self.run_handlers(file, &e);
            e.notes.extend(self.recorder.take(e.id));
            match ran {
                Ok(()) => {
                    e.status = EventStatus::Done;
                    e.message = None;
                }
                Err(message) => {
                    e.status = if e.attempts >= MAX_ATTEMPTS {
                        EventStatus::Failed
                    } else {
                        EventStatus::Queued
                    };
                    e.message = Some(message);
                }
            }
        }
        self.conn().execute(
            "UPDATE device_events
             SET status = ?2, attempts = ?3, message = ?4, updated_ms = ?5, notes = ?6
             WHERE id = ?1",
            params![
                e.id,
                e.status.as_str(),
                e.attempts,
                e.message,
                self.writer.now_ms(),
                serde_json::to_string(&e.notes)?,
            ],
        )?;
        Ok(e)
    }

    /// Every handler of `e`'s event, in file order. The first failure stops the rest.
    fn run_handlers(&self, file: Option<&DeviceFile>, e: &QueuedEvent) -> Result<(), String> {
        let Some(file) = file else {
            return Ok(());
        };
        let value = json!({
            "event": e.event,
            "device": self.device_id(),
            "at": e.at_ms,
            "data": e.data,
        });
        let context = json!({ RUN_EVENT: e.id });
        for h in file.handlers.iter().filter(|h| h.name == e.event) {
            self.run(&h.script, &h.function, value.clone(), context.clone())?;
        }
        Ok(())
    }

    /// Queue a `contact_changed` event for the phone contact of each `person` linked to this
    /// device whose version changed since the last call, with the contact as the phone holds it
    /// now, so the handler carries the person's changes to the contact (D16). Call it after a
    /// sync round. Returns how many it queued.
    ///
    /// It queues nothing while the device may not write contacts, and remembers nothing either,
    /// so the changes in between are taken once the switch is on. The first call only remembers
    /// each person's version: what changed before the phone started syncing is not news.
    pub fn queue_person_changes(&self) -> anyhow::Result<usize> {
        let started = self
            .conn()
            .query_row(
                "SELECT 1 FROM device_runtime WHERE key = 'people_started'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if started && !self.settings().contacts_write {
            return Ok(0);
        }
        let people = self.writer.list(Some("person"))?;
        let seen: std::collections::HashMap<String, String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT id, version FROM device_people")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let slot = format!("device_{}", self.device_id());
        let now = self.writer.now_ms();
        let mut queued = 0;
        for p in &people {
            if seen.get(&p.id) == Some(&p.version) {
                continue;
            }
            self.conn().execute(
                "INSERT INTO device_people (id, version) VALUES (?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET version = excluded.version",
                params![p.id, p.version],
            )?;
            if !started {
                continue;
            }
            let Ok(note) = self.writer.read(&p.id) else {
                continue;
            };
            let doc = writer::doc_to_json(&note.doc);
            let contact_id = doc["blocks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|b| b["kind"] == "dict" && b["labels"] == json!(["device_contacts"]))
                .and_then(|b| b["fields"][&slot].as_str())
                .map(str::to_owned);
            let Some(contact_id) = contact_id else {
                continue;
            };
            // A contact the phone no longer has was deleted there, and its own event unlinks it.
            let Ok(contact) = self.data.contacts_get(&contact_id) else {
                continue;
            };
            if contact.is_object() {
                self.enqueue("contact_changed", contact, now)?;
                queued += 1;
            }
        }
        if !started {
            self.conn().execute(
                "INSERT INTO device_runtime (key, value) VALUES ('people_started', '1')
                 ON CONFLICT (key) DO NOTHING",
                [],
            )?;
        }
        Ok(queued)
    }

    /// Run command `command`, as `GET /api/devices/{id}/commands` lists it: `{ id, name, payload,
    /// ... }`. It runs only when the device file declares it and the settings enable it;
    /// otherwise it fails without running. Blocks while it runs.
    pub fn run_command(&self, command: &Json) -> CommandReport {
        self.run_command_with(command, &mut |_| {})
    }

    /// [`DeviceRuntime::run_command`], telling `progress` how a long command is going: an
    /// `import_history` reports its [`ImportReport`] after each page it reads, as JSON.
    ///
    /// `import_history` is built in: its script function answers the import to run, by default
    /// the command's payload, `{ kinds, since, until }`, and the runtime then runs it
    /// ([`DeviceRuntime::import_history`]). It reports `done` with the import's report, or
    /// `failed` with the report and a `message` when nothing could be read at all.
    pub fn run_command_with(
        &self,
        command: &Json,
        progress: &mut dyn FnMut(&Json),
    ) -> CommandReport {
        let id = command["id"].as_str().unwrap_or_default().to_owned();
        let name = command["name"].as_str().unwrap_or_default();
        let failed = |message: String| CommandReport {
            id: id.clone(),
            done: false,
            result: Json::String(message),
        };
        let file = match self.device_file() {
            Ok(f) => f,
            Err(e) => return failed(format!("{e:#}")),
        };
        let Some(hook) = file.as_ref().and_then(|f| f.command(name)) else {
            return failed(format!(
                "this device does not accept command {name:?}: devices/{}.wcl does not declare it",
                self.platform
            ));
        };
        if !self.settings().commands.iter().any(|c| c == name) {
            return failed(format!(
                "command {name:?} is not enabled for this device; enable it in Settings > Devices"
            ));
        }
        let value = json!({
            "id": id,
            "command": name,
            "payload": command.get("payload").cloned().unwrap_or(Json::Null),
        });
        let ran = self.run(&hook.script, &hook.function, value, Json::Null);
        match ran {
            Ok(asked) if name == IMPORT_COMMAND => self.import_command(&id, &asked, progress),
            Ok(result) => CommandReport {
                id: id.clone(),
                done: true,
                result,
            },
            Err(message) => failed(message),
        }
    }

    /// Run the import an `import_history` script function asked for.
    fn import_command(
        &self,
        id: &str,
        asked: &Json,
        progress: &mut dyn FnMut(&Json),
    ) -> CommandReport {
        let failed = |result: Json| CommandReport {
            id: id.to_owned(),
            done: false,
            result,
        };
        let req = match ImportRequest::from_json(asked) {
            Ok(req) => req,
            Err(message) => return failed(Json::String(message)),
        };
        match self.import_history(&req, &mut |r| progress(&r.to_json())) {
            Ok(report) => {
                let mut result = report.to_json();
                result["message"] = Json::String(report.summary());
                if report.nothing_read() {
                    failed(result)
                } else {
                    CommandReport {
                        id: id.to_owned(),
                        done: true,
                        result,
                    }
                }
            }
            Err(e) => failed(Json::String(format!("{e:#}"))),
        }
    }

    /// The device file in force: the vault's, or the built-in default when the vault has none.
    fn device_file(&self) -> anyhow::Result<Option<DeviceFile>> {
        let text = match self.writer.read_device_file(&self.platform) {
            Ok(text) => text,
            Err(writer::Error::Missing(_)) => match writer::default_device_file(&self.platform) {
                Some(text) => text.to_owned(),
                None => return Ok(None),
            },
            Err(e) => return Err(e.into()),
        };
        Ok(Some(DeviceFile::parse(&self.platform, &text)?))
    }

    /// Run `function` of script `script` on `value`, as a device run with `context`.
    fn run(
        &self,
        script: &str,
        function: &str,
        value: Json,
        context: Json,
    ) -> Result<Json, String> {
        let source = match self.writer.read_script(script) {
            Ok(source) => source,
            Err(e) => writer::default_device_script(script)
                .map(str::to_owned)
                .ok_or_else(|| e.to_string())?,
        };
        let mut job = Job::new(Kind::Device, script, &source, function);
        job.args = vec![value];
        job.context = context;
        self.host.run(job).result.map_err(|e| e.to_string())
    }
}

fn event_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedEvent> {
    let data: String = r.get(2)?;
    let status: String = r.get(4)?;
    let notes: String = r.get(7)?;
    Ok(QueuedEvent {
        id: r.get(0)?,
        event: r.get(1)?,
        data: serde_json::from_str(&data).unwrap_or(Json::Null),
        at_ms: r.get(3)?,
        status: EventStatus::parse(&status),
        attempts: r.get(5)?,
        message: r.get(6)?,
        notes: serde_json::from_str(&notes).unwrap_or_default(),
    })
}

/// The member of a handler run's [`Job::context`] naming the queued event it runs for.
const RUN_EVENT: &str = "device_event";

/// The local vault as device scripts reach it: the writer's, noting each note a handler run
/// creates against the queued event the run is for.
struct Recorder {
    writer: Arc<Writer>,
    /// Queued event id to the notes created for it, until its pass takes them.
    created: Mutex<HashMap<i64, Vec<String>>>,
}

impl Recorder {
    fn take(&self, event: i64) -> Vec<String> {
        self.created
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&event)
            .unwrap_or_default()
    }
}

impl Vault for Recorder {
    fn get(&self, id: &str) -> Result<Json, String> {
        Vault::get(&*self.writer, id)
    }

    fn query(&self, text: &str, max_rows: usize) -> Result<Vec<Json>, String> {
        Vault::query(&*self.writer, text, max_rows)
    }

    fn query_page(&self, text: &str, size: usize, cursor: Option<String>) -> Result<Json, String> {
        Vault::query_page(&*self.writer, text, size, cursor)
    }

    fn search(&self, text: &str, limit: usize, archived: Archived) -> Result<Vec<Json>, String> {
        Vault::search(&*self.writer, text, limit, archived)
    }

    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String> {
        let saved = Vault::create(&*self.writer, type_name, note, author)?;
        let event = current_job(|j| j.context.get(RUN_EVENT).and_then(Json::as_i64)).flatten();
        if let (Some(event), Some(id)) = (event, saved.get("id").and_then(Json::as_str)) {
            self.created
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(event)
                .or_default()
                .push(id.to_owned());
        }
        Ok(saved)
    }

    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String> {
        Vault::update(&*self.writer, id, note, version, author)
    }

    fn delete(&self, id: &str, version: &str, author: &Author) -> Result<Json, String> {
        Vault::delete(&*self.writer, id, version, author)
    }

    fn restore(&self, id: &str, author: &Author) -> Result<Json, String> {
        Vault::restore(&*self.writer, id, author)
    }

    fn set_archived(&self, id: &str, archived: bool, author: &Author) -> Result<Json, String> {
        Vault::set_archived(&*self.writer, id, archived, author)
    }

    fn links(&self, id: &str) -> Result<Vec<Json>, String> {
        Vault::links(&*self.writer, id)
    }

    fn backlinks(&self, id: &str) -> Result<Vec<Json>, String> {
        Vault::backlinks(&*self.writer, id)
    }

    fn assets(&self) -> Result<Vec<Json>, String> {
        Vault::assets(&*self.writer)
    }

    fn asset(&self, id: &str) -> Result<(Json, Vec<u8>), String> {
        Vault::asset(&*self.writer, id)
    }

    fn upload_asset(&self, filename: &str, bytes: Vec<u8>) -> Result<Json, String> {
        Vault::upload_asset(&*self.writer, filename, bytes)
    }

    fn types(&self) -> Result<Vec<Json>, String> {
        Vault::types(&*self.writer)
    }

    fn taxonomies(&self) -> Result<Vec<Json>, String> {
        Vault::taxonomies(&*self.writer)
    }

    fn add_term(&self, taxonomy: &str, path: &str, author: &Author) -> Result<Json, String> {
        Vault::add_term(&*self.writer, taxonomy, path, author)
    }

    fn daily(&self, date: &str, author: &Author) -> Result<Json, String> {
        Vault::daily(&*self.writer, date, author)
    }

    fn secret(&self, name: &str) -> Result<Option<String>, String> {
        Vault::secret(&*self.writer, name)
    }

    fn notify(&self, source: &str, title: &str, body: &str, note: &str) -> Result<Json, String> {
        Vault::notify(&*self.writer, source, title, body, note)
    }

    fn device_command(
        &self,
        source: &str,
        device: &str,
        name: &str,
        payload: Json,
    ) -> Result<Json, String> {
        Vault::device_command(&*self.writer, source, device, name, payload)
    }

    fn protected(&self) -> Vec<PathBuf> {
        Vault::protected(&*self.writer)
    }

    fn now_ms(&self) -> i64 {
        Vault::now_ms(&*self.writer)
    }
}

/// A number with its spacing and punctuation taken out, for comparing.
fn digits(number: &str) -> String {
    number
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect()
}

/// Why `event` with payload `data` is dropped before its handlers run, or `None` (D25).
fn dropped(settings: &DeviceSettings, event: &str, data: &Json) -> Option<String> {
    let text = |v: &Json, k: &str| v.get(k).and_then(Json::as_str).unwrap_or("").to_owned();
    // Every party the event names: the sender or other party, and each recipient of a group MMS.
    let mut parties = vec![data];
    if let Some(to) = data.get("to").and_then(Json::as_array) {
        parties.extend(to);
    }
    let mut numbers = Vec::new();
    let mut contacts = Vec::new();
    for p in parties {
        numbers.push(text(p, "number"));
        numbers.push(text(p, "number_raw"));
        if let Some(c) = p.get("contact") {
            contacts.push(text(c, "id"));
            contacts.push(text(c, "name"));
        }
    }
    match event {
        "contact_changed" | "contact_deleted" => {
            contacts.push(text(data, "contact_id"));
            contacts.push(text(data, "name"));
            if let Some(phones) = data.get("phones").and_then(Json::as_array) {
                numbers.extend(phones.iter().filter_map(Json::as_str).map(str::to_owned));
            }
        }
        "rcs_received" => contacts.push(text(data, "sender")),
        _ => {}
    }
    let thread = text(data, "thread_ref");
    for x in &settings.exclusions {
        let value = x.value.trim();
        let hit = match x.kind {
            ExclusionKind::Number => {
                let want = digits(value);
                !want.is_empty() && numbers.iter().any(|n| digits(n) == want)
            }
            ExclusionKind::Contact => contacts
                .iter()
                .any(|c| !c.is_empty() && (c == value || c.to_lowercase() == value.to_lowercase())),
            ExclusionKind::Thread => !thread.is_empty() && thread == value,
        };
        if hit {
            let kind = match x.kind {
                ExclusionKind::Contact => "contact",
                ExclusionKind::Number => "number",
                ExclusionKind::Thread => "thread",
            };
            return Some(format!("{kind} {value:?} is on the exclusion list"));
        }
    }
    if matches!(event, "sms_received" | "sms_sent") && !settings.skip_pattern.is_empty() {
        // The pattern was checked when it was set; one that does not compile skips nothing.
        if let Ok(re) = regex::Regex::new(&settings.skip_pattern)
            && re.is_match(&text(data, "body"))
        {
            return Some("the body matches the skip pattern".to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(exclusions: Json) -> DeviceSettings {
        serde_json::from_value(json!({ "exclusions": exclusions })).unwrap()
    }

    #[test]
    fn exclusions_match_numbers_contacts_and_threads() {
        let sms = json!({
            "number": "+61400000001",
            "number_raw": "0400 000 001",
            "contact": { "id": "c7", "name": "Sam Lee" },
            "thread_ref": "t3",
            "body": "hi",
        });
        for (kind, value) in [
            ("number", "+61 400 000 001"),
            ("number", "0400-000-001"),
            ("contact", "c7"),
            ("contact", "sam lee"),
            ("thread", "t3"),
        ] {
            let s = settings(json!([{ "kind": kind, "value": value }]));
            assert!(
                dropped(&s, "sms_received", &sms).is_some(),
                "{kind} {value}"
            );
        }
        let s = settings(json!([{ "kind": "number", "value": "+61400000002" }]));
        assert_eq!(dropped(&s, "sms_received", &sms), None);

        // Any recipient of a group MMS, an RCS sender's name, a contact's own id.
        let mms = json!({ "number": "+1", "to": [{ "number": "+61400000002", "number_raw": "" }] });
        assert!(dropped(&s, "mms_received", &mms).is_some());
        let s = settings(json!([{ "kind": "contact", "value": "Sam Lee" }]));
        assert!(dropped(&s, "rcs_received", &json!({ "sender": "Sam Lee" })).is_some());
        let s = settings(json!([{ "kind": "contact", "value": "c7" }]));
        assert!(dropped(&s, "contact_deleted", &json!({ "contact_id": "c7" })).is_some());
    }

    #[test]
    fn the_skip_pattern_drops_only_sms() {
        let s = DeviceSettings::default();
        let code = json!({ "body": "Your verification code is 482913" });
        assert!(dropped(&s, "sms_received", &code).is_some());
        assert!(dropped(&s, "sms_sent", &code).is_some());
        assert_eq!(
            dropped(
                &s,
                "share_received",
                &json!({ "text": "verification code 1234" })
            ),
            None
        );
        assert_eq!(
            dropped(&s, "sms_received", &json!({ "body": "Dinner at 7?" })),
            None
        );
        let none: DeviceSettings = serde_json::from_value(json!({ "skip_pattern": "" })).unwrap();
        assert_eq!(dropped(&none, "sms_received", &code), None);
    }
}
