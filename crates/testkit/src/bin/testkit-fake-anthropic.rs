//! `testkit-fake-anthropic`: run a [`testkit::FakeAnthropic`] with its defaults on a free port
//! and print one JSON line, `{ "base_url", "api_key" }`. The browser tests (S5) start it and point
//! the `ai_settings` secret's `endpoints.anthropic` at it. It runs until its standard input closes
//! or it is killed.
//!
//! Each line on standard input is a command, answered by one JSON line on standard output once it
//! has taken effect, `{ "ok": "<command>" }` or `{ "error": "..." }`:
//!
//! | Command | Effect |
//! |---------|--------|
//! | `push <json>` | Queue the answer to the next `POST /v1/messages`: `{ "text"?: string, "tools"?: [{ "id", "name", "input" }] }`, a message of that text and those tool calls, ending `tool_use` when it has tools |
//! | `requests` | Answers `{ "ok": "requests", "count": <n> }`, the `POST /v1/messages` requests so far |

use std::io::{BufRead, Write};

use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic};

fn main() {
    let fake = FakeAnthropic::start();
    println!(
        "{}",
        json!({ "base_url": fake.base_url(), "api_key": fake.api_key() })
    );
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let answer = match command(&fake, line) {
            Ok(extra) => {
                let mut a = json!({ "ok": line.split(' ').next().unwrap_or_default() });
                if let (Some(a), Some(e)) = (a.as_object_mut(), extra.as_object()) {
                    a.extend(e.clone());
                }
                a
            }
            Err(e) => json!({ "error": e }),
        };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{answer}");
        let _ = out.flush();
    }
}

fn command(fake: &FakeAnthropic, line: &str) -> Result<Value, String> {
    let (name, arg) = line.split_once(' ').unwrap_or((line, ""));
    match name {
        "push" => {
            let spec: Value =
                serde_json::from_str(arg).map_err(|e| format!("push takes JSON: {e}"))?;
            let mut m = AnthropicMessage::new();
            if let Some(text) = spec["text"].as_str() {
                m = m.with_text(text);
            }
            for t in spec["tools"].as_array().into_iter().flatten() {
                let (Some(id), Some(tool)) = (t["id"].as_str(), t["name"].as_str()) else {
                    return Err("each tool needs an id and a name".into());
                };
                m = m.tool_use(id, tool, t["input"].clone());
            }
            fake.push(m);
            Ok(json!({}))
        }
        "requests" => Ok(json!({ "count": fake.message_requests().len() })),
        _ => Err(format!("unknown command {name:?}")),
    }
}
