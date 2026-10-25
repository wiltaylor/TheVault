//! `testkit-fake-openai`: run a [`testkit::FakeOpenAi`] on a free port and print one JSON line,
//! `{ "base_url", "api_key" }`. The browser tests (S5) start it and point an AI account at it.
//! It runs until its standard input closes or it is killed.
//!
//! The one argument picks the mode:
//!
//! - `chat_completions`: an OpenAI-compatible service, answering `POST /v1/chat/completions` and
//!   `GET /v1/models`, and 404 to `/v1/responses`. Point an `openai_compatible` account's base
//!   URL at `<base_url>/v1`, with `api_key` as its key.
//! - `plan`: the ChatGPT plan, answering `POST /v1/responses` and `GET /v1/models` to any bearer
//!   token but `api_key`, each streamed answer carrying the plan usage headers of accounts A13.
//!   Point `ai_settings.endpoints.chatgpt` at `base_url` and store a signed-in
//!   `ai_account_chatgpt_<id>` whose access token is not `api_key`.
//!
//! Each line on standard input is a command, answered by one JSON line on standard output once it
//! has taken effect, `{ "ok": "<command>" }` or `{ "error": "..." }`. The commands `push` and
//! `requests` are those of `testkit-fake-anthropic`:
//!
//! | Command | Effect |
//! |---------|--------|
//! | `push <json>` | Queue the answer to the next inference request: `{ "text"?: string, "tools"?: [{ "id", "name", "input" }] }`, an answer of that text and those tool calls; `"usage": [input, output]` sets the tokens it reports; `"delay_ms"` waits that long before each streamed chunk |
//! | `requests` | Answers `{ "ok": "requests", "count": <n> }`, the inference requests so far |
//! | `last_headers` | Answers `{ "ok": "last_headers", "headers": { <lowercased name>: <value> } }`, the headers of the last request of any kind, or `{}` before one |
//! | `plan_usage <json>` | The used percents later plan answers report: `[primary, secondary]`, or `null` for no usage headers. The default is `[12.5, 40]` |

use std::io::{BufRead, Write};

use serde_json::{Value, json};
use testkit::{FakeOpenAi, OpenAiResponse};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let fake = match mode.as_str() {
        "chat_completions" => FakeOpenAi::builder().chat_completions().start(),
        "plan" => FakeOpenAi::start(),
        other => {
            eprintln!("testkit-fake-openai: the mode is chat_completions or plan, not {other:?}");
            std::process::exit(2);
        }
    };
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

fn command(fake: &FakeOpenAi, line: &str) -> Result<Value, String> {
    let (name, arg) = line.split_once(' ').unwrap_or((line, ""));
    match name {
        "push" => {
            let spec: Value =
                serde_json::from_str(arg).map_err(|e| format!("push takes JSON: {e}"))?;
            let mut r = OpenAiResponse::new();
            if let Some(text) = spec["text"].as_str() {
                r = r.with_text(text);
            }
            for t in spec["tools"].as_array().into_iter().flatten() {
                let (Some(id), Some(tool)) = (t["id"].as_str(), t["name"].as_str()) else {
                    return Err("each tool needs an id and a name".into());
                };
                r = r.function_call(id, tool, t["input"].clone());
            }
            if let Some([input, output]) = spec["usage"].as_array().map(Vec::as_slice) {
                r = r.usage(input.as_u64().unwrap_or(0), output.as_u64().unwrap_or(0));
            }
            if let Some(ms) = spec["delay_ms"].as_u64() {
                r = r.delay(std::time::Duration::from_millis(ms));
            }
            fake.push(r);
            Ok(json!({}))
        }
        "requests" => {
            let count = fake
                .requests()
                .iter()
                .filter(|r| r.method == "POST")
                .count();
            Ok(json!({ "count": count }))
        }
        "last_headers" => {
            let headers = fake
                .requests()
                .last()
                .map_or_else(|| json!({}), |r| json!(r.headers.clone()));
            Ok(json!({ "headers": headers }))
        }
        "plan_usage" => {
            let spec: Value =
                serde_json::from_str(arg).map_err(|e| format!("plan_usage takes JSON: {e}"))?;
            let usage = match spec.as_array().map(Vec::as_slice) {
                Some([p, s]) => Some((p.as_f64().unwrap_or(0.0), s.as_f64().unwrap_or(0.0))),
                _ if spec.is_null() => None,
                _ => return Err("plan_usage takes [primary, secondary] or null".into()),
            };
            fake.set_plan_usage(usage);
            Ok(json!({}))
        }
        _ => Err(format!("unknown command {name:?}")),
    }
}
