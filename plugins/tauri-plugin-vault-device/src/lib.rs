//! TheVault's Android device plugin (note 01).
//!
//! The Kotlin half in `android/` will hold the SMS, call log, contacts, notification listener,
//! UnifiedPush and WorkManager code. Background Kotlin code runs where no webview, and maybe no
//! Tauri runtime, exists, so it reaches the Rust core through plain JNI rather than Tauri
//! commands (D17). [`jni`] holds those entry points.

use tauri::Runtime;
use tauri::plugin::{Builder, TauriPlugin};

pub mod jni;

/// The Kotlin package of the plugin's Android library.
#[cfg(target_os = "android")]
const ANDROID_PACKAGE: &str = "dev.wiltaylor.thevault.device";

/// The plugin, registered on the app's builder.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("vault-device")
        .setup(|_app, api| {
            #[cfg(target_os = "android")]
            api.register_android_plugin(ANDROID_PACKAGE, "VaultDevicePlugin")?;
            #[cfg(not(target_os = "android"))]
            let _ = api;
            Ok(())
        })
        .build()
}
