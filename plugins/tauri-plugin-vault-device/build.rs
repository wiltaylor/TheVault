/// The commands onboarding and Settings > Devices call; each forwards to the Kotlin plugin in
/// `android/`.
const COMMANDS: &[&str] = &[
    "open_capture_setup",
    "capture_status",
    "import_history",
    "import_status",
    "check_now",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
