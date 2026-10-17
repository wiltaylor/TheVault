/// The commands onboarding, Settings > Devices and Settings > General call; each forwards to the Kotlin plugin in
/// `android/`.
const COMMANDS: &[&str] = &[
    "open_capture_setup",
    "capture_status",
    "capture_health",
    "open_capture_fix",
    "import_history",
    "import_status",
    "check_now",
    "update_status",
    "check_for_updates",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
