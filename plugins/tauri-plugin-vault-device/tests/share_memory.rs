//! A large shared file goes into the local vault without the app holding it (ticket 142, D19).
//! The only test in its binary, so nothing else moves the process's peak memory.

use tauri_plugin_vault_device::share::{Shared, SharedFile, store};

/// The process's peak resident memory, in bytes, from `/proc/self/status`.
fn peak_rss() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    let kb: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kb * 1024
}

#[test]
fn a_300_mb_shared_video_is_stored_without_the_app_holding_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let server = embedded::Embedded::open(embedded::Config {
        vault: dir.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    let writer = server.writer();
    let copies = tempfile::TempDir::new().unwrap();
    let path = copies.path().join("copy");
    let size: u64 = 300 * 1024 * 1024;
    // Sparse: 300 MB on disk costs nothing, but reading it whole would.
    std::fs::File::create(&path).unwrap().set_len(size).unwrap();
    let shared = Shared {
        files: vec![SharedFile {
            path: path.clone(),
            filename: "clip.mp4".into(),
            mime: "video/mp4".into(),
        }],
        ..Shared::default()
    };
    let before = peak_rss();
    let stored = store(writer, &shared).unwrap();
    let grew = peak_rss().saturating_sub(before);
    assert_eq!(stored[0]["size"], size);
    assert!(
        grew < 64 * 1024 * 1024,
        "storing the share grew the process by {grew} bytes"
    );
    assert!(!path.exists());
    let id = stored[0]["asset"].as_str().unwrap();
    let kept = dir.path().join(format!("assets/{id}/clip.mp4"));
    assert_eq!(std::fs::metadata(kept).unwrap().len(), size);
}
