//! JNI entry points that Kotlin calls with no webview (D17).
//!
//! Each function is named for the Kotlin `external fun` it implements, in
//! `dev.wiltaylor.thevault.device.VaultCore`. They are exported from whichever library links this
//! crate: the app's `libapp_lib.so` on a phone, or a host build of this crate in the JVM tests.

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jstring;

/// What `VaultCore.ping(message)` answers: the core's name and version, then the message.
pub fn pong(message: &str) -> String {
    format!(
        "{} {}: {message}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )
}

/// `VaultCore.ping(message: String): String`, a round trip from Kotlin into Rust and back. A
/// failed conversion leaves the JNI exception pending and returns null, which Kotlin throws.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_wiltaylor_thevault_device_VaultCore_ping<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    message: JString<'local>,
) -> jstring {
    let message: String = match env.get_string(&message) {
        Ok(message) => message.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match env.new_string(pong(&message)) {
        Ok(answer) => answer.into_raw(),
        Err(_) => std::ptr::null_mut(),
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
