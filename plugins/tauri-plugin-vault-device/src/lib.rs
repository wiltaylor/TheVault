//! TheVault's Android device plugin (note 01).
//!
//! The Kotlin half in `android/` holds onboarding and pairing, the token in the Android
//! Keystore, the background sync worker and the share target; later the SMS, call log, contacts,
//! notification listener and UnifiedPush code. Background Kotlin code runs where no webview, and
//! maybe no Tauri runtime, exists, so it reaches the Rust core through plain JNI rather than Tauri
//! commands (D17). [`jni`] holds those entry points and [`core`] the core they share with the
//! app's webview.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

pub mod core;
pub mod jni;
pub mod pair;
pub mod share;

/// The Kotlin package of the plugin's Android library.
#[cfg(target_os = "android")]
const ANDROID_PACKAGE: &str = "dev.wiltaylor.thevault.device";

/// The plugin, registered on the app's builder. Its setup opens the process's core on the vault
/// in the app's data folder, where the Kotlin side opens it too.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("vault-device")
        .setup(|app, api| {
            // Android's data folder is the app's own (`Context.getDataDir`); elsewhere it is
            // shared, so the app's folder inside it.
            #[cfg(target_os = "android")]
            let data = app.path().data_dir()?;
            #[cfg(not(target_os = "android"))]
            let data = app.path().app_data_dir()?;
            app.manage(core::open(&core::vault_dir(&data))?);
            #[cfg(target_os = "android")]
            api.register_android_plugin(ANDROID_PACKAGE, "VaultDevicePlugin")?;
            #[cfg(not(target_os = "android"))]
            let _ = api;
            Ok(())
        })
        .build()
}
