use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

#[test]
fn size_diagnostics_are_local_and_advisory() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".vault")).unwrap();
    let path = dir.path().join(".vault/state.db");
    let state = state::State::open(&path, None).unwrap();
    state.record_version("mail", "email", &state::Author::Token("fixture".into()), "é").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_vault"))
        .arg("--vault").arg(dir.path())
        .args(["state-size", "--warning-bytes", "1"])
        .output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["history"][0]["content_bytes"], 2);
    assert_eq!(report["history"][0]["versions"], 1);
    assert!(report["warning"].as_str().unwrap().contains("backup"));
    assert_eq!(state.history("mail").unwrap().len(), 1);
}

#[test]
fn prune_is_opt_in_and_reports_content_bytes() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".vault")).unwrap();
    let path = dir.path().join(".vault/state.db");
    let state = state::State::open(&path, None).unwrap();
    for id in ["thread", "email"] {
        for _ in 0..10 {
            state
                .record_version(id, "email", &state::Author::Token("test".into()), "é")
                .unwrap();
        }
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE versions SET note_type = 'agent_thread' WHERE note_id = 'thread'",
            [],
        )
        .unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_vault"))
            .arg("--vault")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let preview = run(&["prune-thread-history", "--keep", "3"]);
    assert_eq!(preview["rows"], 7);
    assert_eq!(preview["bytes"], 14);
    assert_eq!(preview["threads"][0]["id"], "thread");
    assert_eq!(state.history("thread").unwrap().len(), 10);
    let applied = run(&["prune-thread-history", "--keep", "3", "--apply"]);
    assert_eq!(applied["rows"], 7);
    assert_eq!(state.history("thread").unwrap().len(), 3);
    assert_eq!(state.history("email").unwrap().len(), 10);
    assert_eq!(run(&["prune-thread-history"])["rows"], 0);
}
