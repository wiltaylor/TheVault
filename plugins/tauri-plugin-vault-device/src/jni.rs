//! JNI entry points that Kotlin calls with no webview (D17).
//!
//! Each function is named for the Kotlin `external fun` it implements, in
//! `dev.wiltaylor.thevault.device.VaultCore`. They are exported from whichever library links this
//! crate: the app's `libapp_lib.so` on a phone, or a host build of this crate in the JVM tests.
//!
//! Values cross as strings, JSON where they are structured. A failure throws
//! `java.lang.IllegalStateException` with the error's message, and a panic is caught and thrown
//! the same way rather than unwinding into the JVM.
//!
//! The other way, the device module reaches the phone through [`JniDeviceData`], which calls the
//! Kotlin `DeviceBridge` passed to `VaultCore.openDevice`, including the pages of history an
//! import reads (`history`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use embedded::device::{DeviceData, ImportReport, QueuedEvent};
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString};
use jni::sys::{jboolean, jlong, jstring};
use jni::{JNIEnv, JavaVM};
use serde_json::{Value as Json, json};

use crate::core::{self, Core, Link};
use crate::notify::PushTarget;
use crate::pair::{self, Secret};
use crate::share::Shared;

/// What `VaultCore.ping(message)` answers: the core's name and version, then the message.
pub fn pong(message: &str) -> String {
    format!(
        "{} {}: {message}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )
}

/// A Kotlin string, or `None` for null.
fn arg(env: &mut JNIEnv, s: &JString) -> anyhow::Result<Option<String>> {
    if s.is_null() {
        return Ok(None);
    }
    Ok(Some(env.get_string(s)?.into()))
}

fn required(env: &mut JNIEnv, s: &JString, what: &str) -> anyhow::Result<String> {
    arg(env, s)?.ok_or_else(|| anyhow::anyhow!("{what} is null"))
}

/// Run `f` and answer its string, or throw its error and answer null.
fn answer<'local>(
    env: &mut JNIEnv<'local>,
    f: impl FnOnce(&mut JNIEnv<'local>) -> anyhow::Result<Option<String>>,
) -> jstring {
    let result = catch_unwind(AssertUnwindSafe(|| f(env))).unwrap_or_else(|panic| {
        let what = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "a panic".to_owned());
        Err(anyhow::anyhow!("the Rust core panicked: {what}"))
    });
    match result {
        Ok(None) => std::ptr::null_mut(),
        Ok(Some(s)) => env
            .new_string(s)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(e) => {
            if !env.exception_check().unwrap_or(false) {
                let _ = env.throw_new("java/lang/IllegalStateException", format!("{e:#}"));
            }
            std::ptr::null_mut()
        }
    }
}

fn core() -> anyhow::Result<std::sync::Arc<Core>> {
    core::current().ok_or_else(|| anyhow::anyhow!("the vault is not open: call VaultCore.open"))
}

/// `VaultCore.ping(message: String): String`, a round trip from Kotlin into Rust and back.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_ping<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    message: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        Ok(Some(pong(&required(env, &message, "message")?)))
    })
}

/// `VaultCore.open(vault: String): String`: open the vault folder, once per process. Answers the
/// folder.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_open<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    vault: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let vault = required(env, &vault, "vault")?;
        let core = core::open(Path::new(&vault))?;
        Ok(Some(core.vault().display().to_string()))
    })
}

/// `VaultCore.pair(serverUrl, password, code, name): String`: pair with the home server by
/// password or by pairing code, whichever is not null. Answers the pairing as JSON
/// (`{server_url, device_id, device_name, token}`). It makes a network request: call it off the
/// main thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_pair<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    server_url: JString<'local>,
    password: JString<'local>,
    code: JString<'local>,
    name: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let url = required(env, &server_url, "serverUrl")?;
        let name = required(env, &name, "name")?;
        let secret = match (arg(env, &password)?, arg(env, &code)?) {
            (Some(p), None) => Secret::Password(p),
            (None, Some(c)) => Secret::Code(c),
            _ => anyhow::bail!("pair with either a password or a pairing code"),
        };
        let paired = pair::pair(&url, &secret, &name)?;
        Ok(Some(serde_json::to_string(&paired)?))
    })
}

/// `VaultCore.link(link: String?)`: hand the core the pairing as JSON
/// (`{server_url, token, device_id, device_name, cache_mb}`), or null when not paired.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_link<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    link: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let link = match arg(env, &link)? {
            Some(json) => Some(serde_json::from_str::<Link>(&json)?),
            None => None,
        };
        core()?.set_link(link);
        Ok(None)
    })
}

/// `VaultCore.kick()`: ask the foreground loop for a round once writes settle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_kick<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| {
        core()?.kick();
        Ok(None)
    })
}

/// `VaultCore.foreground(on: Boolean)`: the app came to the front or left it, so the core
/// follows the home server's live events, or stops.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_foreground<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    on: jboolean,
) -> jstring {
    answer(&mut env, |_| {
        core()?.set_foreground(on != 0);
        Ok(None)
    })
}

/// `VaultCore.sync(): String`: run one round now and answer its report as JSON. It blocks for
/// the whole round: call it from a worker thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_sync<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| {
        let report = core()?.sync_now()?;
        Ok(Some(core::report_json(&report).to_string()))
    })
}

/// `VaultCore.share(shared: String): String`: store a share (`{text, subject, files: [{path,
/// filename, mime}]}`) and run it as a `share_received` event ([`Core::share`]). Answers
/// `{event, status, message, notes}` as JSON: `event` is null when the share fell back to a note
/// written directly.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_share<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    shared: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let shared: Shared = serde_json::from_str(&required(env, &shared, "shared")?)?;
        Ok(Some(serde_json::to_string(&core()?.share(&shared)?)?))
    })
}

/// `VaultCore.queued(): String`: the notes waiting for the home server, as JSON.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_queued<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| Ok(Some(core()?.queued()?.to_string())))
}

/// `VaultCore.status(): String`: paired or not, notes waiting, and the last round, as JSON.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_status<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| Ok(Some(core()?.status()?.to_string())))
}

/// `VaultCore.runCommands(): String`: fetch the commands server scripts queued for this device,
/// run each the device file declares and its settings enable, and report every result to the home
/// server. Answers what ran as a JSON list of `{ id, name, status, result }`. It makes network
/// requests: call it off the main thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_runCommands<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| Ok(Some(core()?.run_commands()?.to_string())))
}

/// `VaultCore.importHistory(request: String, progress: ImportProgress?): String`: import the
/// phone's history as `request` says, `{ kinds, since, until }` as JSON ([`Core::import_history`]).
/// `progress.update(report)` hears the report as JSON as it goes, on the calling thread. Answers
/// the final report as JSON. It blocks for the whole import: call it from a worker thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_importHistory<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    request: JString<'local>,
    progress: JObject<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let request: Json = serde_json::from_str(&required(env, &request, "request")?)?;
        let core = core()?;
        let mut hear = |r: &ImportReport| {
            if progress.is_null() {
                return;
            }
            let Ok(text) = env.new_string(r.to_json().to_string()) else {
                return;
            };
            let _ = env.call_method(
                &progress,
                "update",
                "(Ljava/lang/String;)V",
                &[(&text).into()],
            );
            // A listener that threw is not the import's problem.
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            let _ = env.delete_local_ref(text);
        };
        let report = core.import_history(&request, &mut hear)?;
        Ok(Some(report.to_json().to_string()))
    })
}

/// `VaultCore.noted(deviceRef: String): String`: `"true"` when the vault holds the `message` or
/// `call` note of `deviceRef`, archived or not, else `"false"`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_noted<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    device_ref: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let device_ref = required(env, &device_ref, "deviceRef")?;
        Ok(Some(core()?.noted(&device_ref)?.to_string()))
    })
}

/// A queued event as Kotlin reads it, with its payload when `data` is set.
pub fn event_json(e: &QueuedEvent, data: bool) -> Json {
    let mut out = json!({
        "id": e.id,
        "event": e.event,
        "at": e.at_ms,
        "status": e.status.as_str(),
        "attempts": e.attempts,
        "message": e.message,
    });
    if data {
        out["data"] = e.data.clone();
    }
    out
}

/// `VaultCore.openDevice(deviceId: String, bridge: DeviceBridge)`: open the device runtime of
/// `deviceId` on the open vault, whose device module reaches the phone through `bridge`. Safe to
/// call again; another device id reopens the runtime on the same queue.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_openDevice<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    device_id: JString<'local>,
    bridge: JObject<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let device_id = required(env, &device_id, "deviceId")?;
        if bridge.is_null() {
            anyhow::bail!("bridge is null");
        }
        let vm = env.get_java_vm()?;
        let bridge = env.new_global_ref(&bridge)?;
        core()?.device(&device_id, || Arc::new(JniDeviceData { vm, bridge }))?;
        Ok(None)
    })
}

/// `VaultCore.enqueue(event: String, data: String, atMs: Long): Long`: store a device event, its
/// payload as JSON text, in the durable queue. Answers its id in the queue.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_enqueue<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    event: JString<'local>,
    data: JString<'local>,
    at_ms: jlong,
) -> jlong {
    let mut id = -1;
    answer(&mut env, |env| {
        let event = required(env, &event, "event")?;
        let data: Json = serde_json::from_str(&required(env, &data, "data")?)?;
        id = core()?.runtime()?.enqueue(&event, data, at_ms)?;
        Ok(None)
    });
    id
}

/// `VaultCore.runPending(): String`: run every queued event's handlers. Answers the events the
/// pass took, as a JSON list of `{ id, event, at, status, attempts, message }`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_runPending<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| {
        let events = core()?.runtime()?.run_pending()?;
        Ok(Some(
            Json::from_iter(events.iter().map(|e| event_json(e, false))).to_string(),
        ))
    })
}

/// `VaultCore.events(): String`: every event in the queue, oldest first, as a JSON list of
/// `{ id, event, at, status, attempts, message, data }`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_events<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| {
        let events = core()?.runtime()?.events()?;
        Ok(Some(
            Json::from_iter(events.iter().map(|e| event_json(e, true))).to_string(),
        ))
    })
}

/// `VaultCore.newFailures(): String`: the device events that failed for good since the last call,
/// oldest first, each answered once, as a JSON list of `{ id, event, at, status, attempts,
/// message, who }`, for the phone to announce.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_newFailures<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| {
        let events = core()?.runtime()?.new_failures()?;
        Ok(Some(
            Json::from_iter(events.iter().map(QueuedEvent::problem_json)).to_string(),
        ))
    })
}

/// `VaultCore.setCaptureHealth(health: String)`: the phone's capture health, `{ problems, ... }`
/// as JSON, which the next round sends to the home server when it changed (ticket 91).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_setCaptureHealth<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    health: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let health: Json = serde_json::from_str(&required(env, &health, "health")?)?;
        core()?.set_capture_health(health)?;
        Ok(None)
    })
}

/// `VaultCore.putAsset(filename: String, bytes: ByteArray): String`: store a file in the local
/// vault, such as an MMS part. Answers `{ asset, filename, size }`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_putAsset<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    filename: JString<'local>,
    bytes: JByteArray<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let filename = required(env, &filename, "filename")?;
        let bytes = env.convert_byte_array(&bytes)?;
        let asset = core()?
            .server()
            .writer()
            .put_asset(&filename, bytes)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Some(
            json!({ "asset": asset.id, "filename": asset.filename, "size": asset.size })
                .to_string(),
        ))
    })
}

/// `VaultCore.unreadNotifications(): String`: the home server's unread notifications, newest
/// first, as a JSON array. It makes a network request: call it off the main thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_unreadNotifications<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    answer(&mut env, |_| {
        Ok(Some(serde_json::to_string(
            &core()?.unread_notifications()?,
        )?))
    })
}

/// `VaultCore.markRead(id: String): String`: mark a notification read on the home server.
/// Answers `true`, or `false` when the server no longer has it. It makes a network request.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_markRead<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    id: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let id = required(env, &id, "id")?;
        Ok(Some(core()?.mark_read(&id)?.to_string()))
    })
}

/// `VaultCore.setPush(endpoint: String?, p256dh: String?, auth: String?)`: register the
/// UnifiedPush endpoint and its WebPush keys with the home server, or clear it when `endpoint` is
/// null. It makes a network request.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_setPush<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    endpoint: JString<'local>,
    p256dh: JString<'local>,
    auth: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let target = match arg(env, &endpoint)? {
            Some(endpoint) => Some(PushTarget {
                endpoint,
                p256dh: required(env, &p256dh, "p256dh")?,
                auth: required(env, &auth, "auth")?,
            }),
            None => None,
        };
        core()?.set_push(target.as_ref())?;
        Ok(None)
    })
}

/// The phone's data and actions for the device module, through the Kotlin `DeviceBridge`:
/// `call(method: String, args: String): String`, where `args` is a JSON map of the arguments and
/// the answer is `{ "ok": value }` or `{ "err": message }`.
pub struct JniDeviceData {
    vm: JavaVM,
    bridge: GlobalRef,
}

impl JniDeviceData {
    fn call(&self, method: &str, args: Json) -> Result<Json, String> {
        let answer = self
            .call_bridge(method, &args)
            .map_err(|e| format!("the device could not answer {method}: {e:#}"))?;
        let answer: Json = serde_json::from_str(&answer)
            .map_err(|e| format!("the device answered {method} with bad JSON: {e}"))?;
        match answer.get("err") {
            Some(err) => Err(err.as_str().unwrap_or("the device refused").to_owned()),
            None => Ok(answer.get("ok").cloned().unwrap_or(Json::Null)),
        }
    }

    fn call_bridge(&self, method: &str, args: &Json) -> anyhow::Result<String> {
        // Script runs happen on the host's own threads, which Java has never seen.
        let mut env = self.vm.attach_current_thread_permanently()?;
        let method = env.new_string(method)?;
        let args = env.new_string(args.to_string())?;
        let answer = env.call_method(
            &self.bridge,
            "call",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[(&method).into(), (&args).into()],
        );
        if env.exception_check()? {
            env.exception_describe()?;
            env.exception_clear()?;
            anyhow::bail!("the bridge threw");
        }
        let answer = JString::from(answer?.l()?);
        Ok(env.get_string(&answer)?.into())
    }

    fn list(&self, method: &str, args: Json) -> Result<Vec<Json>, String> {
        match self.call(method, args)? {
            Json::Array(rows) => Ok(rows),
            other => Err(format!(
                "the device answered {method} with {other}, not a list"
            )),
        }
    }
}

impl DeviceData for JniDeviceData {
    fn sms_list(&self, filter: &Json) -> Result<Vec<Json>, String> {
        self.list("sms_list", json!({ "filter": filter }))
    }

    fn sms_get(&self, device_ref: &str) -> Result<Json, String> {
        self.call("sms_get", json!({ "device_ref": device_ref }))
    }

    fn calls_list(&self, filter: &Json) -> Result<Vec<Json>, String> {
        self.list("calls_list", json!({ "filter": filter }))
    }

    fn contacts_list(&self, filter: &Json) -> Result<Vec<Json>, String> {
        self.list("contacts_list", json!({ "filter": filter }))
    }

    fn contacts_get(&self, contact_id: &str) -> Result<Json, String> {
        self.call("contacts_get", json!({ "contact_id": contact_id }))
    }

    fn contacts_upsert(&self, contact: &Json) -> Result<Json, String> {
        self.call("contacts_upsert", json!({ "contact": contact }))
    }

    fn contacts_delete(&self, contact_id: &str) -> Result<(), String> {
        self.call("contacts_delete", json!({ "contact_id": contact_id }))
            .map(|_| ())
    }

    fn sms_send(&self, to: &str, body: &str) -> Result<Json, String> {
        self.call("sms_send", json!({ "to": to, "body": body }))
    }

    fn show_notification(&self, title: &str, body: &str, note: &str) -> Result<(), String> {
        self.call(
            "show_notification",
            json!({ "title": title, "body": body, "note": note }),
        )
        .map(|_| ())
    }

    fn history(&self, request: &Json) -> Result<Json, String> {
        self.call("history", request.clone())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pong_names_the_core_and_echoes_the_message() {
        assert_eq!(
            super::pong("hello"),
            format!(
                "tauri-plugin-vault-device {}: hello",
                env!("CARGO_PKG_VERSION")
            )
        );
    }
}
