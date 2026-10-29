//! `testkit-fake-google`: run a [`testkit::FakeGoogle`] with its defaults on a free port and print
//! one JSON line, `{ "base_url", "host", "endpoints", "auth" }`, where `endpoints` is the
//! `endpoints` object of the `google_settings` secret and `auth` the fake consent page. The
//! browser tests (S5) start it and point the `google_settings` secret at it. It runs until its standard input closes or it is killed.
//!
//! Each line on standard input is a command, answered by one JSON line on standard output once it
//! has taken effect: `{ "ok": "<command>" }`, or `{ "error": "..." }` for a line it does not know.
//!
//! | Command | Effect |
//! |---------|--------|
//! | `gmail-delay <ms>` | [`testkit::FakeGoogle::set_gmail_delay`]: each Gmail request waits `<ms>` milliseconds |
//! | `gmail-delay off` | Gmail answers at once, releasing any request still waiting |
//! | `calendar-delay <ms>` | [`testkit::FakeGoogle::set_calendar_delay`], the same for Calendar |
//! | `calendar-delay off` | Calendar answers at once, releasing any request still waiting |
//! | `add-message <json>` | [`testkit::FakeGoogle::add_message`] with the message spec `<json>`, on one line |

use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let google = testkit::FakeGoogle::start();
    println!(
        "{}",
        serde_json::json!({
            "base_url": google.base_url(),
            "host": google.host(),
            "endpoints": google.endpoints(),
            "auth": google.url(testkit::fake_google::AUTH_PATH),
        })
    );
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let answer = match command(&google, line) {
            Ok(()) => serde_json::json!({ "ok": line }),
            Err(e) => serde_json::json!({ "error": e }),
        };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{answer}");
        let _ = out.flush();
    }
}

/// Carry out one command line.
fn command(google: &testkit::FakeGoogle, line: &str) -> Result<(), String> {
    let (name, arg) = line.split_once(' ').unwrap_or((line, ""));
    if name == "add-message" {
        let spec: serde_json::Value =
            serde_json::from_str(arg).map_err(|e| format!("add-message takes a JSON spec: {e}"))?;
        google.add_message(spec);
        return Ok(());
    }
    let delay = match arg.trim() {
        "off" => None,
        ms => Some(Duration::from_millis(ms.parse().map_err(|_| {
            format!("{name} takes milliseconds or off, not {ms:?}")
        })?)),
    };
    match name {
        "gmail-delay" => google.set_gmail_delay(delay),
        "calendar-delay" => google.set_calendar_delay(delay),
        _ => return Err(format!("unknown command {name:?}")),
    }
    Ok(())
}
