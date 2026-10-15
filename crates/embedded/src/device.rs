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
//! A vault without `devices/<platform>.wcl` runs the built-in default device file, and a script
//! the device file names that the vault lacks runs from the built-in defaults (D15).

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};
use schema::{DEVICE_EVENTS, DeviceFile};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use writer::Writer;

use script_core::{DeviceAccess, DeviceExtensions, Host, Job, Kind, Limits, Vault};
pub use script_core::{DeviceData, Switches};

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
    updated_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS device_events_status ON device_events (status, id);

CREATE TABLE IF NOT EXISTS device_runtime (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
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
}

impl EventStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EventStatus::Queued => "queued",
            EventStatus::Done => "done",
            EventStatus::Failed => "failed",
            EventStatus::Skipped => "skipped",
        }
    }

    fn parse(s: &str) -> EventStatus {
        match s {
            "done" => EventStatus::Done,
            "failed" => EventStatus::Failed,
            "skipped" => EventStatus::Skipped,
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
    access: Arc<DeviceAccess>,
    platform: String,
    settings: RwLock<DeviceSettings>,
    queue: Mutex<Connection>,
    /// Held for a whole pass, so two passes never run the same event.
    pass: Mutex<()>,
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
            config.data,
            settings.switches(),
        ));
        let vault: Arc<dyn Vault> = writer.clone();
        let host = Host::with_extensions(
            vault,
            1,
            Limits::default(),
            &DeviceExtensions(access.clone()),
        );
        Ok(DeviceRuntime {
            writer,
            host,
            access,
            platform: config.platform,
            settings: RwLock::new(settings),
            queue: Mutex::new(conn),
            pass: Mutex::new(()),
        })
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
                "SELECT id, event, data, at_ms, status, attempts, message FROM device_events
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
            "SELECT id, event, data, at_ms, status, attempts, message FROM device_events
             ORDER BY id",
        )?;
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
                "DELETE FROM device_events WHERE status IN ('done', 'skipped') AND updated_ms < ?1",
                [now - RETENTION_MS],
            )?;
            let mut stmt = conn.prepare(
                "SELECT id, event, data, at_ms, status, attempts, message FROM device_events
                 WHERE status = 'queued' ORDER BY id",
            )?;
            let rows = stmt.query_map([], event_row)?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let settings = self.settings();
        let file = self.device_file()?;
        let mut out = Vec::with_capacity(queued.len());
        for mut e in queued {
            if let Some(why) = dropped(&settings, &e.event, &e.data) {
                e.status = EventStatus::Skipped;
                e.message = Some(why);
            } else {
                e.attempts += 1;
                match self.run_handlers(file.as_ref(), &e) {
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
                "UPDATE device_events SET status = ?2, attempts = ?3, message = ?4, updated_ms = ?5
                 WHERE id = ?1",
                params![
                    e.id,
                    e.status.as_str(),
                    e.attempts,
                    e.message,
                    self.writer.now_ms()
                ],
            )?;
            out.push(e);
        }
        Ok(out)
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
        for h in file.handlers.iter().filter(|h| h.name == e.event) {
            self.run(&h.script, &h.function, value.clone())?;
        }
        Ok(())
    }

    /// Run command `command`, as `GET /api/devices/{id}/commands` lists it: `{ id, name, payload,
    /// ... }`. It runs only when the device file declares it and the settings enable it;
    /// otherwise it fails without running. Blocks while it runs.
    pub fn run_command(&self, command: &Json) -> CommandReport {
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
        match self.run(&hook.script, &hook.function, value) {
            Ok(result) => CommandReport {
                id: id.clone(),
                done: true,
                result,
            },
            Err(message) => failed(message),
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

    /// Run `function` of script `script` on `value`, as a device run.
    fn run(&self, script: &str, function: &str, value: Json) -> Result<Json, String> {
        let source = match self.writer.read_script(script) {
            Ok(source) => source,
            Err(e) => writer::default_device_script(script)
                .map(str::to_owned)
                .ok_or_else(|| e.to_string())?,
        };
        let mut job = Job::new(Kind::Device, script, &source, function);
        job.args = vec![value];
        self.host.run(job).result.map_err(|e| e.to_string())
    }
}

fn event_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedEvent> {
    let data: String = r.get(2)?;
    let status: String = r.get(4)?;
    Ok(QueuedEvent {
        id: r.get(0)?,
        event: r.get(1)?,
        data: serde_json::from_str(&data).unwrap_or(Json::Null),
        at_ms: r.get(3)?,
        status: EventStatus::parse(&status),
        attempts: r.get(5)?,
        message: r.get(6)?,
    })
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
