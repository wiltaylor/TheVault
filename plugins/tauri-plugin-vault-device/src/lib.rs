//! TheVault's Android device plugin (note 01).
//!
//! The Kotlin half in `android/` holds onboarding and pairing, the token in the Android
//! Keystore, the background sync worker, the share target, SMS, MMS, call log and contacts
//! capture with its capture setup screen (D7), and the UnifiedPush receiver that mirrors the home
//! server's notifications (D9); later the notification listener code. Background Kotlin code
//! runs where no webview, and
//! maybe no Tauri runtime, exists, so it reaches the Rust core through plain JNI rather than Tauri
//! commands (D17). [`jni`] holds those entry points and [`core`] the core they share with the
//! app's webview.
//!
//! The commands here are for the webview: `open_capture_setup` and `capture_status`. They
//! forward to the Kotlin plugin, and fail anywhere but Android.

use serde_json::{Value as Json, json};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime};

pub mod core;
pub mod jni;
pub mod notify;
pub mod pair;
pub mod share;

/// The Kotlin package of the plugin's Android library.
#[cfg(target_os = "android")]
const ANDROID_PACKAGE: &str = "dev.wiltaylor.thevault.device";

/// The Kotlin plugin, once registered.
#[cfg(target_os = "android")]
struct Mobile<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Run Kotlin command `command` with `payload`.
fn run_kotlin<R: Runtime>(
    app: &AppHandle<R>,
    command: &str,
    payload: Json,
) -> Result<Json, String> {
    #[cfg(target_os = "android")]
    {
        let mobile = app
            .try_state::<Mobile<R>>()
            .ok_or("the device plugin is not registered")?;
        mobile
            .0
            .run_mobile_plugin::<Json>(command, payload)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, payload);
        Err(format!("{command} works only on Android"))
    }
}

/// Open the capture setup screen: each permission in turn, then the first-run import (D7).
#[tauri::command]
async fn open_capture_setup<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    run_kotlin(&app, "openCaptureSetup", json!({})).map(|_| ())
}

/// `{ setupDone, deviceId, permissions }`: whether capture is set up, and what Android granted.
#[tauri::command]
async fn capture_status<R: Runtime>(app: AppHandle<R>) -> Result<Json, String> {
    run_kotlin(&app, "captureStatus", json!({}))
}

/// The plugin, registered on the app's builder. Its setup opens the process's core on the vault
/// in the app's data folder, where the Kotlin side opens it too.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("vault-device")
        .invoke_handler(tauri::generate_handler![open_capture_setup, capture_status])
        .setup(|app, api| {
            // Android's data folder is the app's own (`Context.getDataDir`); elsewhere it is
            // shared, so the app's folder inside it.
            #[cfg(target_os = "android")]
            let data = app.path().data_dir()?;
            #[cfg(not(target_os = "android"))]
            let data = app.path().app_data_dir()?;
            app.manage(core::open(&core::vault_dir(&data))?);
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin(ANDROID_PACKAGE, "VaultDevicePlugin")?;
                app.manage(Mobile(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = api;
            Ok(())
        })
        .build()
}
