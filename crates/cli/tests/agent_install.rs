//! Connecting an AI agent (ticket 131, D11, D32): the one-line command from Settings > API tokens
//! run in a shell with a temporary `HOME`, against a test server whose clients folder holds this
//! build of the CLI, and `vault` reading the `cli.env` the script writes.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;
use testkit::TestServer;

const VAULT: &str = env!("CARGO_BIN_EXE_vault");

/// `vault` with only `HOME` and `PATH` set, so whatever it reaches comes from `cli.env`.
fn bare(bin: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .unwrap()
}

fn json_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn write_cli_env(config: &Path, url: &str, token: &str) {
    let dir = config.join("thevault");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("cli.env"),
        format!("VAULT_URL={url}\nVAULT_TOKEN={token}\n"),
    )
    .unwrap();
}

#[test]
fn cli_env_is_read_when_the_environment_is_unset_and_env_and_flags_win() {
    let server = TestServer::start();
    let other = TestServer::start();
    let config = TempDir::new().unwrap();
    write_cli_env(config.path(), &server.base_url(), server.admin_token());
    let run = |args: &[&str], env: &[(&str, &str)]| {
        let mut cmd = Command::new(VAULT);
        cmd.args(args)
            .env_remove("VAULT_URL")
            .env_remove("VAULT_TOKEN")
            .env("XDG_CONFIG_HOME", config.path())
            .env("HOME", config.path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };

    // Nothing set: the file gives both.
    let out = run(&["list"], &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // The environment's URL wins: the other server does not know the file's token.
    let out = run(&["list"], &[("VAULT_URL", &other.base_url())]);
    assert_eq!(
        json_of(&out)["status"],
        401,
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // And the environment's token, with the file's URL.
    let out = run(&["list"], &[("VAULT_TOKEN", other.admin_token())]);
    assert_eq!(json_of(&out)["status"], 401);
    let out = run(&["list"], &[("VAULT_TOKEN", "tv_wrong")]);
    assert_eq!(json_of(&out)["status"], 401);
    // Flags win over both.
    let out = run(
        &[
            "list",
            "--url",
            &other.base_url(),
            "--token",
            other.admin_token(),
        ],
        &[
            ("VAULT_URL", "http://127.0.0.1:1"),
            ("VAULT_TOKEN", "tv_wrong"),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = run(&["list", "--token", "tv_wrong"], &[]);
    assert_eq!(json_of(&out)["status"], 401);

    // With no XDG_CONFIG_HOME, the file is under ~/.config.
    let home = TempDir::new().unwrap();
    write_cli_env(
        &home.path().join(".config"),
        &server.base_url(),
        server.admin_token(),
    );
    let out = bare(Path::new(VAULT), home.path(), &["list"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // `vault --help` says so.
    let help = Command::new(VAULT).arg("--help").output().unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("thevault/cli.env"), "{help}");
}

/// A test server whose clients folder holds this build of `vault` as the CLI for this machine.
fn server_with_cli() -> (TestServer, TempDir) {
    let scratch = TempDir::new().unwrap();
    let build = scratch.path().join(format!(
        "vault-{}-linux-{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    ));
    std::fs::copy(VAULT, &build).unwrap();
    let clients = scratch.path().join("clients");
    let out = Command::new(VAULT)
        .args(["clients", "publish", "--dir"])
        .arg(&clients)
        .arg(&build)
        .env("HOME", scratch.path())
        .env("XDG_CONFIG_HOME", scratch.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let server = TestServer::start_with_args(
        |_| {},
        &[std::ffi::OsStr::new("--clients"), clients.as_os_str()],
    );
    (server, scratch)
}

fn install_command(server: &TestServer, name: &str) -> String {
    let res = reqwest::blocking::Client::new()
        .post(server.url("/api/agent-codes"))
        .bearer_auth(server.admin_token())
        .json(&json!({ "name": name, "scope": "write" }))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 201);
    let body: Value = res.json().unwrap();
    body["command"].as_str().unwrap().to_owned()
}

/// Runs `command` in `sh` the way a user pastes it, with only `HOME` and `PATH` set.
fn paste(command: &str, home: &Path) -> (bool, String) {
    let out = Command::new("sh")
        .args(["-c", command])
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn token_in(env_file: &Path) -> String {
    std::fs::read_to_string(env_file)
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("VAULT_TOKEN="))
        .unwrap()
        .to_owned()
}

#[test]
fn the_one_line_install_sets_up_the_cli_its_config_and_the_skill() {
    let (server, _scratch) = server_with_cli();
    let skill = Command::new(VAULT)
        .args(["skill", "--raw"])
        .output()
        .unwrap()
        .stdout;

    // A machine with Claude Code.
    let home = TempDir::new().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    let command = install_command(&server, "claude");
    assert!(command.starts_with(&format!(
        "curl -fsSL {}/agent/install.sh | sh -s -- ",
        server.base_url()
    )));
    let (ok, said) = paste(&command, home.path());
    assert!(ok, "{said}");

    let bin = home.path().join(".local/bin/vault");
    assert_eq!(std::fs::read(&bin).unwrap(), std::fs::read(VAULT).unwrap());
    assert!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o111 != 0);
    let env_file = home.path().join(".config/thevault/cli.env");
    assert_eq!(
        std::fs::metadata(&env_file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = std::fs::read_to_string(&env_file).unwrap();
    assert!(
        text.contains(&format!("VAULT_URL={}\n", server.base_url())),
        "{text}"
    );
    let token = token_in(&env_file);
    assert!(token.starts_with("tv_"), "{text}");
    assert!(!said.contains(&token), "the token was printed:\n{said}");
    assert_eq!(
        std::fs::read(home.path().join(".config/thevault/SKILL.md")).unwrap(),
        skill
    );
    assert_eq!(
        std::fs::read(home.path().join(".claude/skills/thevault/SKILL.md")).unwrap(),
        skill
    );
    assert!(
        said.contains(
            "Before using the vault, read ~/.config/thevault/SKILL.md; the vault CLI is configured."
        ),
        "{said}"
    );

    // The installed CLI works with nothing in the environment.
    let health = bare(&bin, home.path(), &["health"]);
    assert_eq!(json_of(&health)["status"], "ok");
    let list = bare(&bin, home.path(), &["list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stdout)
    );
    assert!(
        json_of(&list)["notes"].is_array(),
        "{}",
        String::from_utf8_lossy(&list.stdout)
    );

    // Again without a code: the CLI and skill are refreshed and the token kept.
    std::fs::write(
        home.path().join(".claude/skills/thevault/SKILL.md"),
        "stale",
    )
    .unwrap();
    let script = format!("curl -fsSL {}/agent/install.sh | sh", server.base_url());
    let (ok, said) = paste(&script, home.path());
    assert!(ok, "{said}");
    assert_eq!(token_in(&env_file), token);
    assert_eq!(
        std::fs::read(home.path().join(".claude/skills/thevault/SKILL.md")).unwrap(),
        skill
    );
    // A used code is refused and changes nothing; a new one replaces the token.
    let (ok, said) = paste(&command, home.path());
    assert!(!ok, "{said}");
    assert!(said.contains("used or expired"), "{said}");
    assert_eq!(token_in(&env_file), token);
    let (ok, said) = paste(&install_command(&server, "claude"), home.path());
    assert!(ok, "{said}");
    let rotated = token_in(&env_file);
    assert_ne!(rotated, token);
    assert!(!said.contains(&rotated), "the token was printed:\n{said}");
    assert!(bare(&bin, home.path(), &["list"]).status.success());

    // A machine without Claude Code gets only the config folder's copy.
    let plain = TempDir::new().unwrap();
    let (ok, said) = paste(&install_command(&server, "codex"), plain.path());
    assert!(ok, "{said}");
    assert_eq!(
        std::fs::read(plain.path().join(".config/thevault/SKILL.md")).unwrap(),
        skill
    );
    assert!(!plain.path().join(".claude").exists());
    assert!(
        bare(
            &plain.path().join(".local/bin/vault"),
            plain.path(),
            &["list"]
        )
        .status
        .success()
    );

    // VAULT_BIN_DIR moves the binary.
    let elsewhere = TempDir::new().unwrap();
    let (ok, said) = paste(
        &install_command(&server, "other").replace("| sh", "| VAULT_BIN_DIR=\"$HOME/bin\" sh"),
        elsewhere.path(),
    );
    assert!(ok, "{said}");
    assert!(elsewhere.path().join("bin/vault").is_file());
    assert!(said.contains("to PATH"), "{said}");
}
