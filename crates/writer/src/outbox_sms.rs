//! An Outbox SMS's result, from the phone that sent it (outbox X6).
//!
//! The `outbox_send` handler queues `send_sms` for the item's device with `{ to, body, outbox }`
//! and records the command's id as the item's `sent_ref`, leaving the item `sending`. The phone
//! sends the text, waits for Android to say whether it went, and reports the command `done` with
//! `{ device_ref: "sms:<_id>" }` (the sent row in the phone's SMS store) or `failed` with the
//! reason. [`Writer::finish_outbox_sms`] then records the send, with `sent_ref` the
//! `device_ref` and `sent_at` now, or sets the item `failed` with the error, and when the sent
//! copy has reached the vault already links it as `result` and removes the item for good, since
//! a sent item leaves the Outbox (outbox X14). The copy is found:
//!
//! - by its `device_ref`, the `message` note the phone's capture of the sent SMS made;
//! - when the phone is an older app, which reports no `device_ref`, by the outgoing SMS to that
//!   number with that text sent within [`COPY_WINDOW_MS`].
//!
//! A copy that arrives later is linked by the phone's capture (`device_messages.ws`), and the
//! server's `outbox_send` handler then removes the item; one that never arrives leaves the item
//! `sending` until the sweep removes it 24 hours after the send ([`Writer::sweep_outbox`]).
//!
//! The writes are authored `handler outbox_send`, so the X2 rules hold: only an item still
//! `sending` for this command changes, and only to `failed` or by recording the send.

use script_core::Vault as _;
use serde_json::{Value as Json, json};
use state::{Author, CommandStatus, DeviceCommand};

use crate::Writer;
use crate::builtin::{MESSAGE_TYPE, OUTBOX_TYPE};
use crate::devices::SEND_SMS;
use crate::time;

/// How far apart an older app's sent copy and the send may be, in milliseconds: ten minutes.
pub const COPY_WINDOW_MS: i64 = 10 * 60 * 1000;

/// The handler the result's writes are authored as: the Outbox's send handler.
const SENDER: &str = "outbox_send";

/// The most recent outgoing SMS an older app's copy is looked for among.
const RECENT: usize = 50;

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The ASCII letters and digits of `s`, lower case, and every character beyond ASCII: an SMS
/// compared by what it says, whatever markup, spacing or line breaks the item's body and the
/// phone's copy differ by. `device_messages.ws` compares the same way.
pub fn sms_words(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_alphanumeric() || !c.is_ascii())
        .collect()
}

/// Whether phone numbers `a` and `b` are the same number: their digits equal, or their last nine
/// digits when both have nine or more, so `0400 000 002` is `+61400000002`.
pub fn same_number(a: &str, b: &str) -> bool {
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let (a, b) = (digits(a), digits(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.len() >= 9 && b.len() >= 9 {
        a[a.len() - 9..] == b[b.len() - 9..]
    } else {
        a == b
    }
}

/// The text of the paragraphs of content field `name` of `note` (`{ fields, blocks }`).
fn content_text(note: &Json, name: &str) -> String {
    let mut out = Vec::new();
    for b in note["blocks"].as_array().into_iter().flatten() {
        if b["kind"] == "content"
            && b["labels"]
                .as_array()
                .is_some_and(|l| l.contains(&json!(name)))
        {
            collect_text(&b["blocks"], &mut out);
        }
    }
    out.join("\n")
}

fn collect_text(blocks: &Json, out: &mut Vec<String>) {
    for b in blocks.as_array().into_iter().flatten() {
        if let Some(t) = b["fields"]["text"].as_str() {
            out.push(t.to_owned());
        }
        collect_text(&b["blocks"], out);
    }
}

/// What a failed command's result says: its text, or its `error` or `message`.
fn failure(result: &Json) -> String {
    let text = match result {
        Json::String(s) => s.clone(),
        Json::Null => String::new(),
        other => other["error"]
            .as_str()
            .or_else(|| other["message"].as_str())
            .map_or_else(|| other.to_string(), str::to_owned),
    };
    let text = text.trim();
    if text.is_empty() {
        "the phone gave no reason".to_owned()
    } else {
        text.to_owned()
    }
}

impl Writer {
    /// Finish the Outbox item of `command`, a `send_sms` the phone reported `done` or `failed`
    /// (outbox X6). Any other command, and a command whose item has moved on, is left alone.
    pub fn finish_outbox_sms(&self, command: &DeviceCommand) -> Result<(), String> {
        if command.name != SEND_SMS
            || !matches!(command.status, CommandStatus::Done | CommandStatus::Failed)
        {
            return Ok(());
        }
        let payload: Json = serde_json::from_str(&command.payload).unwrap_or_default();
        let Some(item) = payload["outbox"].as_str().filter(|s| !s.is_empty()) else {
            return Ok(());
        };
        // An item deleted since stays deleted.
        let Ok(note) = self.get(item) else {
            return Ok(());
        };
        let fields = &note["note"]["fields"];
        // Only the send this command made: a retry since queued a command of its own.
        if note["type"] != OUTBOX_TYPE
            || fields["status"] != "sending"
            || fields["sent_ref"] != command.id.as_str()
        {
            return Ok(());
        }
        let version = note["version"].as_str().unwrap_or_default();
        let result: Json = command
            .result
            .as_deref()
            .and_then(|r| serde_json::from_str(r).ok())
            .unwrap_or_default();
        let author = Author::Handler(SENDER.to_owned());
        if command.status == CommandStatus::Failed {
            let error = format!("The phone could not send the SMS: {}", failure(&result));
            self.patch(
                item,
                json!({ "status": "failed", "error": error }),
                version,
                &author,
            )?;
            return Ok(());
        }
        let now = self.now_ms();
        let device_ref = result["device_ref"].as_str().filter(|r| !r.is_empty());
        let mut set = json!({ "sent_at": time::utc(now), "error": null });
        let copy = match device_ref {
            Some(r) => {
                set["sent_ref"] = json!(r);
                self.sms_copy_by_ref(r)?
            }
            None => self.sms_copy_by_text(
                payload["to"].as_str().unwrap_or_default(),
                payload["body"].as_str().unwrap_or_default(),
                now,
            )?,
        };
        if let Some(copy) = &copy {
            set["result"] = json!(copy);
        }
        let saved = self.patch(item, set, version, &author)?;
        if copy.is_some() {
            let version = saved["version"].as_str().unwrap_or_default();
            match self.remove_sent_outbox(item, Some(version), &author) {
                Ok(_) | Err(crate::Error::NotFound(_)) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }

    /// The `message` note of the SMS the phone stored as `device_ref`.
    fn sms_copy_by_ref(&self, device_ref: &str) -> Result<Option<String>, String> {
        let rows = self.query(
            &format!(
                "from {MESSAGE_TYPE} include archived where device_ref == {} limit 1",
                quoted(device_ref)
            ),
            1,
        )?;
        Ok(rows
            .first()
            .and_then(|r| r["id"].as_str())
            .map(str::to_owned))
    }

    /// The outgoing SMS to `to` saying `body`, sent within [`COPY_WINDOW_MS`] of `at`: an older
    /// app's sent copy, which no `device_ref` names.
    fn sms_copy_by_text(&self, to: &str, body: &str, at: i64) -> Result<Option<String>, String> {
        let words = sms_words(body);
        if to.is_empty() || words.is_empty() {
            return Ok(None);
        }
        let rows = self.query(
            &format!(
                "from {MESSAGE_TYPE} include archived where direction == \"out\" && channel == \"sms\" \
                 sort sent desc limit {RECENT}"
            ),
            RECENT,
        )?;
        for row in rows {
            let fields = &row["fields"];
            let sent = fields["sent"]
                .as_str()
                .and_then(schema::parse_datetime)
                .map(|s| (s * 1000.0) as i64);
            if sent.is_none_or(|s| (s - at).abs() > COPY_WINDOW_MS) {
                continue;
            }
            let to_us = fields["to_raw"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
                .any(|n| same_number(n, to));
            if !to_us {
                continue;
            }
            let Some(id) = row["id"].as_str() else {
                continue;
            };
            let Ok(message) = self.get(id) else {
                continue;
            };
            if sms_words(&content_text(&message["note"], "body")) == words {
                return Ok(Some(id.to_owned()));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_match_by_their_digits() {
        assert!(same_number("+61400000002", "0400 000 002"));
        assert!(same_number("+61 400 000 002", "+61400000002"));
        assert!(same_number("1234", "12-34"));
        assert!(!same_number("+61400000002", "+61400000003"));
        assert!(!same_number("", ""));
        assert!(!same_number("1234", "123"));
    }

    #[test]
    fn texts_match_by_their_words() {
        assert_eq!(sms_words("Bins **out**,\n tonight!"), "binsouttonight");
        assert_eq!(
            sms_words("Bins out, tonight"),
            sms_words("bins out tonight")
        );
    }

    #[test]
    fn a_failure_says_why() {
        assert_eq!(failure(&json!("no service")), "no service");
        assert_eq!(failure(&json!({ "error": "radio off" })), "radio off");
        assert_eq!(failure(&Json::Null), "the phone gave no reason");
    }
}
