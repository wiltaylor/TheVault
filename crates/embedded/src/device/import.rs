//! Importing the phone's history again, on demand (D7).
//!
//! Capture reads each provider from a cursor, so an item is read once: a message whose note was
//! deleted, or one capture missed, never comes back by itself. An import reads the providers
//! again over a range of dates, whatever the cursors say, and runs each item as the device event
//! capture would have made of it, so the device file's handlers decide the notes.
//!
//! It never duplicates a note. Before an item is queued, the vault is asked for a `message` or
//! `call` note with the item's `device_ref`, archived ones included; an item that has one is
//! counted as already there and left alone. That check is against the notes, not against the
//! capture cursors or the event queue's record of what ran, so an item whose note was deleted is
//! imported again, and exactly once. The default handlers make the same check, so an item that
//! live capture queued as well is still filed once.
//!
//! Contacts carry no `device_ref`: each contact changed within the range runs as a
//! `contact_changed` event, which the contacts handler settles field by field.
//!
//! RCS messages cannot be imported: Google Messages keeps them to itself, and TheVault only sees
//! them as their notifications arrive (D13).

use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{DeviceRuntime, EventStatus};
use script_core::Vault;

/// The command that asks a phone to import its history.
pub const IMPORT_COMMAND: &str = "import_history";

/// What an import can read, in the order it reads them.
pub const IMPORT_KINDS: [&str; 4] = ["sms", "mms", "calls", "contacts"];

/// Why `rcs` is refused as a kind.
pub const RCS_NOT_IMPORTABLE: &str = "RCS messages cannot be imported: Google Messages keeps them \
     to itself, and TheVault only sees them as their notifications arrive";

/// Items asked of the phone at a time.
const PAGE: usize = 200;

/// The events each kind's items arrive as.
fn kind_events(kind: &str) -> &'static [&'static str] {
    match kind {
        "sms" => &["sms_received", "sms_sent"],
        "mms" => &["mms_received"],
        "calls" => &["call_ended"],
        "contacts" => &["contact_changed"],
        _ => &[],
    }
}

/// The note type that holds the items of `kind`, which carry a `device_ref`.
fn note_type(kind: &str) -> Option<&'static str> {
    match kind {
        "sms" | "mms" => Some("message"),
        "calls" => Some("call"),
        _ => None,
    }
}

/// What to import: some kinds, over a range of item times in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    /// Some of [`IMPORT_KINDS`], in that order.
    pub kinds: Vec<String>,
    /// The earliest item time, or `None` for no bound.
    pub since_ms: Option<i64>,
    /// The latest item time, or `None` for no bound.
    pub until_ms: Option<i64>,
}

/// A time member: RFC 3339 text, milliseconds, or null.
fn time_of(v: &Json, key: &str) -> Result<Option<i64>, String> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{key} must be a time")),
        Some(Json::String(s)) if s.is_empty() => Ok(None),
        Some(Json::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|t| Some(t.timestamp_millis()))
            .map_err(|_| format!("{key} {s:?} is not an RFC 3339 time")),
        Some(other) => Err(format!("{key} must be a time, not {other}")),
    }
}

impl ImportRequest {
    /// Every kind, over all time.
    pub fn everything() -> ImportRequest {
        ImportRequest {
            kinds: IMPORT_KINDS.iter().map(|k| (*k).to_owned()).collect(),
            since_ms: None,
            until_ms: None,
        }
    }

    /// The request in `{ kinds, since, until }`, the `import_history` payload. A missing `kinds`
    /// is every kind; `since` and `until` are RFC 3339 times or milliseconds, and missing or null
    /// for no bound.
    pub fn from_json(v: &Json) -> Result<ImportRequest, String> {
        if !(v.is_object() || v.is_null()) {
            return Err("the import is a map: { kinds, since, until }".into());
        }
        let kinds = match v.get("kinds") {
            None | Some(Json::Null) => IMPORT_KINDS.iter().map(|k| (*k).to_owned()).collect(),
            Some(Json::Array(asked)) => {
                let mut names = Vec::new();
                for k in asked {
                    let Some(k) = k.as_str() else {
                        return Err(format!("kind {k} is not a name"));
                    };
                    if k == "rcs" {
                        return Err(RCS_NOT_IMPORTABLE.into());
                    }
                    if !IMPORT_KINDS.contains(&k) {
                        return Err(format!(
                            "{k:?} cannot be imported: the kinds are {}",
                            IMPORT_KINDS.join(", ")
                        ));
                    }
                    names.push(k);
                }
                if names.is_empty() {
                    return Err("choose at least one kind to import".into());
                }
                IMPORT_KINDS
                    .iter()
                    .filter(|k| names.contains(k))
                    .map(|k| (*k).to_owned())
                    .collect()
            }
            Some(other) => return Err(format!("kinds must be a list, not {other}")),
        };
        let since_ms = time_of(v, "since")?;
        let until_ms = time_of(v, "until")?;
        if let (Some(s), Some(u)) = (since_ms, until_ms)
            && s > u
        {
            return Err("since is after until".into());
        }
        Ok(ImportRequest {
            kinds,
            since_ms,
            until_ms,
        })
    }

    fn holds(&self, at: i64) -> bool {
        self.since_ms.is_none_or(|s| at >= s) && self.until_ms.is_none_or(|u| at <= u)
    }
}

/// How one kind of an import went.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KindReport {
    pub kind: String,
    /// Items the phone holds within the range.
    pub found: u64,
    /// Items whose events ran.
    pub imported: u64,
    /// Notes those events' handlers created.
    pub notes: u64,
    /// Messages and calls that already had a note, and were left alone.
    pub existing: u64,
    /// Items the exclusion list or the skip pattern dropped (D25).
    pub skipped: u64,
    /// Items whose handler failed. Each stays queued for the next pass, as any device event.
    pub failed: u64,
    /// The Android permission the kind needs that is not granted; nothing of the kind was read.
    pub missing_permission: Option<String>,
    /// Why the kind could not be read, or the first handler failure.
    pub error: Option<String>,
    /// Whether the kind is finished.
    pub done: bool,
}

/// How an import went, or is going: each kind in turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub kinds: Vec<KindReport>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Whether every kind is finished.
    pub done: bool,
}

impl ImportReport {
    fn new(req: &ImportRequest) -> ImportReport {
        ImportReport {
            kinds: req
                .kinds
                .iter()
                .map(|k| KindReport {
                    kind: k.clone(),
                    ..KindReport::default()
                })
                .collect(),
            since: req.since_ms,
            until: req.until_ms,
            done: false,
        }
    }

    pub fn to_json(&self) -> Json {
        serde_json::to_value(self).unwrap_or(Json::Null)
    }

    /// The report of `kind`.
    pub fn kind(&self, kind: &str) -> Option<&KindReport> {
        self.kinds.iter().find(|k| k.kind == kind)
    }

    /// Whether no kind could be read at all, for want of a permission or otherwise.
    pub fn nothing_read(&self) -> bool {
        self.kinds
            .iter()
            .all(|k| k.found == 0 && (k.missing_permission.is_some() || k.error.is_some()))
    }

    /// One line on how it went, for a person.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = self
            .kinds
            .iter()
            .map(|k| {
                let label = match k.kind.as_str() {
                    "sms" => "SMS",
                    "mms" => "MMS",
                    "calls" => "calls",
                    "contacts" => "contacts",
                    other => other,
                };
                if let Some(p) = &k.missing_permission {
                    return format!("{label}: not read, Android has not given TheVault {p}");
                }
                if k.found == 0
                    && let Some(e) = &k.error
                {
                    return format!("{label}: not read, {e}");
                }
                let mut s = format!("{label}: {} found, {} new notes", k.found, k.notes);
                if k.existing > 0 {
                    s += &format!(", {} already in the vault", k.existing);
                }
                if k.skipped > 0 {
                    s += &format!(", {} excluded", k.skipped);
                }
                if k.failed > 0 {
                    s += &format!(", {} failed", k.failed);
                }
                s
            })
            .collect();
        parts.join("; ")
    }
}

/// A string literal for a query.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl DeviceRuntime {
    /// Whether the vault holds a note of `type_name` with `device_ref`, archived or not. A note in
    /// the trash does not count.
    pub fn noted_as(&self, type_name: &str, device_ref: &str) -> anyhow::Result<bool> {
        let q = format!(
            "from {type_name} include archived where device_ref == {} limit 1",
            quoted(device_ref)
        );
        let rows = Vault::query(&*self.writer, &q, 1).map_err(anyhow::Error::msg)?;
        Ok(!rows.is_empty())
    }

    /// Whether the vault holds a `message` or `call` note with `device_ref`: a call's ref starts
    /// `call:`, any other is a message's.
    pub fn noted(&self, device_ref: &str) -> anyhow::Result<bool> {
        let type_name = if device_ref.starts_with("call:") {
            "call"
        } else {
            "message"
        };
        self.noted_as(type_name, device_ref)
    }

    /// Import `req` from the phone: read each kind page by page through
    /// [`DeviceData::history`](script_core::DeviceData::history), and run each item that has no
    /// note yet as its device event, now. `progress` hears the report after each page and each
    /// kind. One import runs at a time on a device; another is refused while it runs. It blocks.
    ///
    /// A kind the phone cannot read, for want of a permission or otherwise, says so in its report
    /// and the import goes on with the next kind.
    pub fn import_history(
        &self,
        req: &ImportRequest,
        progress: &mut dyn FnMut(&ImportReport),
    ) -> anyhow::Result<ImportReport> {
        let _one = match self.importing.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                anyhow::bail!("an import is already running on this device")
            }
        };
        let mut report = ImportReport::new(req);
        progress(&report);
        for i in 0..report.kinds.len() {
            let kind = report.kinds[i].kind.clone();
            let mut after = Json::Null;
            loop {
                let ask = json!({
                    "kind": kind,
                    "since": req.since_ms,
                    "until": req.until_ms,
                    "after": after,
                    "limit": PAGE,
                });
                let page = match self.data.history(&ask) {
                    Ok(page) => page,
                    Err(e) => {
                        report.kinds[i].error = Some(e);
                        break;
                    }
                };
                if let Some(p) = page["missing_permission"].as_str() {
                    report.kinds[i].missing_permission = Some(p.to_owned());
                    break;
                }
                let events = page["events"].as_array().cloned().unwrap_or_default();
                for item in &events {
                    self.import_one(&kind, item, req, &mut report.kinds[i])?;
                }
                progress(&report);
                match page.get("next") {
                    Some(next) if !next.is_null() && !events.is_empty() && *next != after => {
                        after = next.clone();
                    }
                    _ => break,
                }
            }
            report.kinds[i].done = true;
            progress(&report);
        }
        report.done = true;
        progress(&report);
        Ok(report)
    }

    /// One item of `kind` from the phone: leave it when its note exists, or run its event now.
    fn import_one(
        &self,
        kind: &str,
        item: &Json,
        req: &ImportRequest,
        k: &mut KindReport,
    ) -> anyhow::Result<()> {
        let event = item["event"].as_str().unwrap_or_default();
        if let Some(at) = item["at"].as_i64()
            && !req.holds(at)
        {
            return Ok(());
        }
        if !kind_events(kind).contains(&event) {
            k.failed += 1;
            k.error
                .get_or_insert_with(|| format!("the phone answered {kind} with event {event:?}"));
            return Ok(());
        }
        k.found += 1;
        let data = item.get("data").cloned().unwrap_or(Json::Null);
        let device_ref = data["device_ref"].as_str().unwrap_or_default();
        if let Some(type_name) = note_type(kind)
            && !device_ref.is_empty()
        {
            // An MMS the phone knew had a note comes as its ref alone: never queue that.
            if item["known"] == true || self.noted_as(type_name, device_ref)? {
                k.existing += 1;
                return Ok(());
            }
        }
        let id = match self.enqueue(event, data, self.writer.now_ms()) {
            Ok(id) => id,
            Err(e) => {
                k.failed += 1;
                k.error.get_or_insert_with(|| format!("{e:#}"));
                return Ok(());
            }
        };
        let Some(e) = self.run_event(id)? else {
            return Ok(());
        };
        match e.status {
            EventStatus::Done => {
                k.imported += 1;
                k.notes += e.notes.len() as u64;
            }
            EventStatus::Skipped => k.skipped += 1,
            EventStatus::Queued | EventStatus::Failed => {
                k.failed += 1;
                if let Some(m) = e.message {
                    k.error.get_or_insert(m);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_take_kinds_in_order_and_times_as_text_or_milliseconds() {
        let r = ImportRequest::from_json(&json!({
            "kinds": ["calls", "sms", "calls"],
            "since": "2026-01-01T00:00:00Z",
            "until": 1_800_000_000_000_i64,
        }))
        .unwrap();
        assert_eq!(r.kinds, ["sms", "calls"]);
        assert_eq!(r.since_ms, Some(1_767_225_600_000));
        assert_eq!(r.until_ms, Some(1_800_000_000_000));
        assert_eq!(
            ImportRequest::from_json(&json!({})).unwrap(),
            ImportRequest::everything()
        );
        assert_eq!(
            ImportRequest::from_json(&Json::Null).unwrap(),
            ImportRequest::everything()
        );
    }

    #[test]
    fn rcs_unknown_kinds_and_backwards_ranges_are_refused() {
        let err = |v: Json| ImportRequest::from_json(&v).unwrap_err();
        assert_eq!(err(json!({ "kinds": ["sms", "rcs"] })), RCS_NOT_IMPORTABLE);
        assert!(err(json!({ "kinds": ["photos"] })).contains("cannot be imported"));
        assert!(err(json!({ "kinds": [] })).contains("at least one"));
        assert!(err(json!({ "since": "yesterday" })).contains("RFC 3339"));
        assert!(err(json!({ "since": 2, "until": 1 })).contains("after until"));
        assert!(err(json!([1])).contains("is a map"));
    }

    #[test]
    fn query_literals_escape_quotes_and_braces() {
        assert_eq!(quoted(r#"a"b\{c}"#), r#""a\"b\\{{c}}""#);
    }
}
