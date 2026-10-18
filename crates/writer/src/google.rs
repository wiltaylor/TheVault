//! The secrets the Google sync keeps its settings and state in (google-sync D21). Each holds a
//! JSON object, except [`ACCOUNT`], which holds the connected address as plain text.
//!
//! | Secret | Written by | Holds |
//! |--------|------------|-------|
//! | [`SETTINGS`] | the Integrations page | `gmail_mode`, `gmail_labels`, `gmail_window_days`, `gmail_backfill_to`, `calendars`, `default_calendar`, `endpoints` |
//! | [`GMAIL_STATE`] | the `google_gmail` task | `history_id`, `backfill_page`, `last_run`, `last_error`, `notified_invalid_grant` |
//! | [`CALENDAR_STATE`] | the `google_calendar` task | `sync_tokens`, `window_end`, `last_run`, `last_error`, `notified_invalid_grant` |
//! | [`ACCOUNT`] | connecting Google | the connected address |
//!
//! `endpoints` is the test seam: an object with `token`, `gmail`, `calendar` and `userinfo`
//! URLs. Absent means Google's own hosts.

use serde_json::{Map, Value, json};

use crate::Result;

/// The sync's settings.
pub const SETTINGS: &str = "google_settings";
/// The Gmail task's cursors and last run.
pub const GMAIL_STATE: &str = "google_gmail_state";
/// The Calendar task's cursors and last run.
pub const CALENDAR_STATE: &str = "google_calendar_state";
/// The connected Google address.
pub const ACCOUNT: &str = "google_account";

/// The settings a vault without [`SETTINGS`] syncs with.
pub fn default_settings() -> Map<String, Value> {
    let Value::Object(m) = json!({
        "gmail_mode": "labels",
        "gmail_labels": [],
        "gmail_window_days": 90,
        "gmail_backfill_to": null,
        "calendars": ["primary"],
        "default_calendar": "primary",
    }) else {
        unreachable!()
    };
    m
}

/// The JSON object in secret `name`: empty when there is none, or it holds something else.
pub fn read_object(state: &state::State, name: &str) -> Result<Map<String, Value>> {
    Ok(match state.get_secret(name)? {
        Some(text) => match serde_json::from_str(&text) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        },
        None => Map::new(),
    })
}

/// Store `value` as the JSON object in secret `name`.
pub fn write_object(state: &state::State, name: &str, value: &Map<String, Value>) -> Result<()> {
    state.set_secret(name, &Value::Object(value.clone()).to_string())?;
    Ok(())
}

/// [`default_settings`] with secret [`SETTINGS`] over it.
pub fn settings(state: &state::State) -> Result<Map<String, Value>> {
    let mut out = default_settings();
    for (k, v) in read_object(state, SETTINGS)? {
        out.insert(k, v);
    }
    Ok(out)
}
