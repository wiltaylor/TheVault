//! JNI entry points that Kotlin calls with no webview (D17).
//!
//! Each function is named for the Kotlin `external fun` it implements, in
//! `dev.wiltaylor.thevault.device.VaultCore`. They are exported from whichever library links this
//! crate: the app's `libapp_lib.so` on a phone, or a host build of this crate in the JVM tests.
//!
//! Values cross as strings, JSON where they are structured. A failure throws
//! `java.lang.IllegalStateException` with the error's message, and a panic is caught and thrown
//! the same way rather than unwinding into the JVM.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jstring;

use crate::core::{self, Core, Link};
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
/// filename}]}`) as a fleeting note and answer its id.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_share<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    shared: JString<'local>,
) -> jstring {
    answer(&mut env, |env| {
        let shared: Shared = serde_json::from_str(&required(env, &shared, "shared")?)?;
        Ok(Some(core()?.share(&shared)?))
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
