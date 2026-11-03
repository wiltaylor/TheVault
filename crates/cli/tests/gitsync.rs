//! `vault gitsync` against a running server (git-sync D18): the real binary, asserting its JSON
//! output, with a bare repo in a temp folder as the remote, over `file://`.

use std::path::Path;
use std::process::Command;

use serde_json::json;
use tempfile::TempDir;
use testkit::TestServer;

use crate::cli::{Vault, s};

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/doc.wcl"), "label = \"Doc\"\n").unwrap();
}

/// A bare repo in a fresh temp folder, and its `file://` URL.
fn bare_repo() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("sync.git");
    let out = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "git init --bare failed: {out:?}");
    let url = format!("file://{}", repo.display());
    (dir, url)
}

/// The branches of the bare repo at `url`.
fn branches(url: &str) -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-remote", "--heads", url])
        .output()
        .unwrap();
    assert!(out.status.success(), "git ls-remote failed: {out:?}");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split('\t').nth(1))
        .map(|r| r.trim_start_matches("refs/heads/").to_owned())
        .collect()
}

#[test]
#[ignore = "needs gs-round"]
fn status_shows_no_secret_and_set_keeps_the_settings_it_was_not_given() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);

    let fresh = vault.ok(&["gitsync", "status"], "");
    assert_eq!(fresh["configured"], false);
    assert_eq!(fresh["running"], false);
    assert_eq!(fresh["token_set"], false);
    assert_eq!(fresh["key_set"], false);

    vault.ok(
        &[
            "gitsync",
            "set",
            "--remote-url",
            "https://github.com/me/vault-sync.git",
            "--auth",
            "https",
            "--export-query",
            "from doc",
            "--import-query",
            "from doc",
            "--name",
            "laptop",
        ],
        "",
    );
    vault.ok(
        &[
            "gitsync",
            "set",
            "--schedule",
            "0 * * * *",
            "--paused",
            "true",
        ],
        "",
    );
    let status = vault.ok(&["gitsync", "status"], "");
    let settings = &status["settings"];
    assert_eq!(
        settings["remote_url"],
        "https://github.com/me/vault-sync.git"
    );
    assert_eq!(settings["auth"], "https");
    assert_eq!(settings["export_query"], "from doc");
    assert_eq!(settings["import_query"], "from doc");
    assert_eq!(settings["name"], "laptop");
    assert_eq!(settings["schedule"], "0 * * * *");
    assert_eq!(settings["paused"], true);
    assert_eq!(settings["https_username"], "x-access-token");

    // An import query may not use `this` (git-sync D23): a 400, and the settings stay.
    let refused = vault.fails(
        &[
            "gitsync",
            "set",
            "--import-query",
            "from doc where id == this",
        ],
        "",
    );
    assert_eq!(refused["status"], 400);
    let again = vault.ok(&["gitsync", "status"], "");
    assert_eq!(again["settings"]["import_query"], "from doc");
}

#[test]
#[ignore = "needs gs-round"]
fn key_generate_prints_the_key_once_and_key_set_reads_it_from_stdin() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);

    let made = vault.ok(&["gitsync", "key", "generate"], "");
    let key = s(&made["key"]).to_owned();
    assert!(key.starts_with("tvsync1-"), "{key}");

    let status = vault.run(&["gitsync", "status"], "");
    assert!(status.ok);
    assert_eq!(status.json["key_set"], true);
    assert!(!status.stdout.contains(&key), "status shows the key");
    let suffix = key.trim_start_matches("tvsync1-");
    assert!(!status.stdout.contains(suffix), "status shows the key");

    // The same key pasted into another instance, with the newline `echo` adds.
    let other = TestServer::start_seeded(seed);
    let other_vault = Vault::against(&other);
    other_vault.ok(&["gitsync", "key", "set"], &format!("{key}\n"));
    assert_eq!(other_vault.ok(&["gitsync", "status"], "")["key_set"], true);

    let malformed = other_vault.fails(&["gitsync", "key", "set"], "not-a-key\n");
    assert_eq!(malformed["status"], 400);
}

#[test]
#[ignore = "needs gs-round"]
fn the_token_and_the_ssh_key_are_stored_but_never_shown() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);

    vault.ok(&["gitsync", "token", "set"], "github_pat_cli_test_123\n");
    let status = vault.run(&["gitsync", "status"], "");
    assert_eq!(status.json["token_set"], true);
    assert!(!status.stdout.contains("github_pat_cli_test_123"));
    vault.ok(&["gitsync", "token", "clear"], "");
    assert_eq!(vault.ok(&["gitsync", "status"], "")["token_set"], false);

    let made = vault.ok(&["gitsync", "ssh-key", "generate"], "");
    let public = s(&made["public_key"]).to_owned();
    assert!(public.starts_with("ssh-ed25519 "), "{public}");
    let status = vault.run(&["gitsync", "status"], "");
    assert_eq!(status.json["ssh_key_set"], true);
    assert_eq!(status.json["ssh_public_key"], public.as_str());
    assert!(!status.stdout.contains("PRIVATE KEY"));

    // A pasted key, read from stdin, that is not an OpenSSH private key is a 400.
    let pasted = vault.fails(&["gitsync", "ssh-key", "set"], "not a key\n");
    assert_eq!(pasted["status"], 400);
}

#[test]
#[ignore = "needs gs-round"]
fn run_waits_for_the_round_and_prints_its_report() {
    let server = TestServer::start_seeded(seed);
    let vault = Vault::against(&server);
    let (_repo, url) = bare_repo();
    vault.ok(&["put", "--type", "doc"], "title = \"Shared\"\n");
    vault.ok(&["gitsync", "key", "generate"], "");
    vault.ok(
        &[
            "gitsync",
            "set",
            "--remote-url",
            &url,
            "--export-query",
            "from doc",
            "--import-query",
            "from doc",
            "--paused",
            "true",
        ],
        "",
    );

    let done = vault.ok(&["gitsync", "run"], "");
    assert_eq!(done["running"], false);
    assert_eq!(done["last_error"], json!(null));
    assert_eq!(done["last_report"]["pushed"], true, "{done}");
    let heads = branches(&url);
    assert_eq!(heads.len(), 1, "{heads:?}");
    assert!(heads[0].starts_with("tv-"), "{heads:?}");

    // Nothing changed, so the next round pushes nothing.
    let again = vault.ok(&["gitsync", "run"], "");
    assert_eq!(again["last_report"]["pushed"], false, "{again}");

    // --no-wait answers as soon as the round starts.
    vault.ok(&["gitsync", "run", "--no-wait"], "");

    // A round that fails, here against a remote that is not there, fails with its error.
    vault.ok(
        &[
            "gitsync",
            "set",
            "--remote-url",
            "file:///nonexistent/thevault-gitsync.git",
        ],
        "",
    );
    // Wait out the --no-wait round before starting another.
    while vault.ok(&["gitsync", "status"], "")["running"] == true {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let failed = vault.fails(&["gitsync", "run"], "");
    assert_eq!(failed["error"], "gitsync");
    assert!(failed["message"].is_string(), "{failed}");

    let gone = vault.ok(&["gitsync", "disconnect"], "");
    assert_eq!(gone["disconnected"], true);
    let status = vault.ok(&["gitsync", "status"], "");
    assert_eq!(status["configured"], false);
    assert_eq!(status["key_set"], false);
}
