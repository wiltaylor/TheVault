/// The commands onboarding calls; each forwards to the Kotlin plugin in `android/`.
const COMMANDS: &[&str] = &["open_capture_setup", "capture_status"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
