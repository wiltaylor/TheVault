//! Commands server scripts queue for the phone, as the phone fetches and answers them (D22).
//!
//! A `device.command` push names a command id and nothing more, and a phone with no push checks
//! every 15 minutes, so both end in [`run`]: refresh the device's settings, which say what is
//! enabled, fetch the pending commands, and for each report `delivered`, run it through the
//! device runtime, and report `done` or `failed` with its result. The runtime refuses a command
//! the device file does not declare or the settings do not enable, and the device module refuses
//! a contact write or an SMS whose switch is off; either way the command is reported `failed`.
//!
//! A command is reported `delivered` before it runs, so one whose run a crash cut short is never
//! run a second time: it expires on the server instead. Sending an SMS twice is worse than not at
//! all. A long command, an `import_history`, reports `delivered` again while it runs, with
//! `{ progress }` as its result, so Settings > Devices can show how far it got.

use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::{Value, json};

use embedded::device::DeviceRuntime;

use crate::core::Link;
use crate::notify::{client, refused, send};

/// How often a long command reports its progress, as `delivered` with `{ progress }`.
const PROGRESS_EVERY: Duration = Duration::from_secs(2);

/// This device's settings from Settings > Devices, as `GET /api/devices` lists them, or `None`
/// when the server no longer lists the device.
pub fn settings(link: &Link) -> anyhow::Result<Option<Value>> {
    let res = send(
        link,
        client()?.get(format!("{}/api/devices", link.server_url)),
    )?;
    if !res.status().is_success() {
        return Err(refused(res));
    }
    let list: Value = res.json()?;
    Ok(list["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["id"] == link.device_id.as_str())
        .map(|d| d["settings"].clone()))
}

/// The commands waiting for this device, oldest first.
pub fn pending(link: &Link) -> anyhow::Result<Vec<Value>> {
    let rb = client()?
        .get(format!(
            "{}/api/devices/{}/commands",
            link.server_url, link.device_id
        ))
        .query(&[("status", "pending")]);
    let res = send(link, rb)?;
    if !res.status().is_success() {
        return Err(refused(res));
    }
    let list: Value = res.json()?;
    Ok(list["commands"].as_array().cloned().unwrap_or_default())
}

/// Report on command `id` with `body`, `{ status, result }`. `Ok(false)` when the server will not
/// take it because the command expired or already finished.
pub fn report(link: &Link, id: &str, body: &Value) -> anyhow::Result<bool> {
    let rb = client()?
        .post(format!(
            "{}/api/devices/{}/commands/{id}/result",
            link.server_url, link.device_id
        ))
        .json(body);
    let res = send(link, rb)?;
    match res.status() {
        s if s.is_success() => Ok(true),
        StatusCode::CONFLICT | StatusCode::NOT_FOUND => Ok(false),
        _ => Err(refused(res)),
    }
}

/// Tell the home server of each device event that failed for good and has not been reported,
/// at `POST /api/devices/{id}/event-failures`, which adds one notification under the bell. Each
/// is reported once: an answer the server will never accept (a 4xx) counts as reported too, and
/// a server that cannot be reached leaves the rest for the next round. Answers how many the
/// server took.
pub fn report_failures(link: &Link, runtime: &DeviceRuntime) -> anyhow::Result<usize> {
    let mut took = 0;
    for e in runtime.unreported_failures()? {
        let rb = client()?
            .post(format!(
                "{}/api/devices/{}/event-failures",
                link.server_url, link.device_id
            ))
            .json(&e.problem_json());
        let res = send(link, rb)?;
        let status = res.status();
        if status.is_success() {
            took += 1;
        } else if status.is_client_error()
            && status != StatusCode::UNAUTHORIZED
            && status != StatusCode::TOO_MANY_REQUESTS
        {
            log::warn!(
                "the home server refused the failure of device event {}: {:#}",
                e.id,
                refused(res)
            );
        } else {
            return Err(refused(res));
        }
        runtime.mark_reported(e.id)?;
    }
    Ok(took)
}

/// Run every pending command of `link`'s device through `runtime`, after taking the device's
/// settings from the server. Answers what ran, `[{ id, name, status, result }]`, in order. It
/// blocks while the commands run.
pub fn run(link: &Link, runtime: &DeviceRuntime) -> anyhow::Result<Value> {
    if let Some(s) = settings(link)?.filter(|s| s.is_object()) {
        runtime.set_settings(&s)?;
    }
    let mut out = Vec::new();
    for command in pending(link)? {
        let Some(id) = command["id"].as_str() else {
            continue;
        };
        if !report(link, id, &json!({ "status": "delivered" }))? {
            continue;
        }
        // A long command, such as an import, reports how it is going as it goes, at most every
        // PROGRESS_EVERY; a report that fails is left for the next one.
        let mut last: Option<Instant> = None;
        let ran = runtime.run_command_with(&command, &mut |progress| {
            if last.is_some_and(|at| at.elapsed() < PROGRESS_EVERY) {
                return;
            }
            last = Some(Instant::now());
            let body = json!({ "status": "delivered", "result": { "progress": progress } });
            if let Err(e) = report(link, id, &body) {
                log::warn!("progress of command {id} not reported: {e:#}");
            }
        });
        let body = ran.body();
        report(link, id, &body)?;
        out.push(json!({
            "id": id,
            "name": command["name"],
            "status": body["status"],
            "result": body["result"],
        }));
    }
    Ok(Value::Array(out))
}
