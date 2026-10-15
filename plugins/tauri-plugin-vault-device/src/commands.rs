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
//! all.

use reqwest::StatusCode;
use serde_json::{Value, json};

use embedded::device::DeviceRuntime;

use crate::core::Link;
use crate::notify::{client, refused, send};

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
        let ran = runtime.run_command(&command);
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
