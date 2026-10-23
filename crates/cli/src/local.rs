//! The commands that work on a vault folder directly rather than through the API: `init`,
//! `passwd` and `reindex` (D28, D37, OQ-S2, OQ-S3).

use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use state::{SecretKey, State};

use crate::Fail;

/// `state.db` inside a vault folder.
pub fn state_db(vault: &Path) -> PathBuf {
    vault.join(".vault").join("state.db")
}

/// Where the key file of `vault_id` lives, from the environment (OQ-S2).
pub fn key_path(vault_id: &str) -> Result<PathBuf, Fail> {
    state::key_file_path(
        vault_id,
        std::env::var_os("VAULT_SECRET_KEY_FILE").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
    .ok_or_else(|| {
        Fail::new(
            "no_config_dir",
            "cannot place the secret key: set XDG_CONFIG_HOME, HOME or VAULT_SECRET_KEY_FILE",
        )
    })
}

/// Open the `state.db` of an existing vault.
pub fn open_state(vault: &Path) -> Result<State, Fail> {
    let db = state_db(vault);
    if !db.is_file() {
        return Err(Fail::new(
            "not_a_vault",
            format!(
                "{} is not a vault: it has no .vault/state.db",
                vault.display()
            ),
        ));
    }
    State::open(&db, None).map_err(Fail::from_display("state"))
}

/// `vault init`: create the layout, the key file and the first password.
pub fn init(dir: &Path, starter: bool, password_stdin: bool) -> Result<Value, Fail> {
    if state_db(dir).exists() {
        return Err(Fail::new(
            "already_a_vault",
            format!("{} is already a vault", dir.display()),
        ));
    }
    let password = read_password(password_stdin)?;
    let vault = store::Vault::init(dir).map_err(Fail::from_display("store"))?;
    let root = vault.root().to_path_buf();
    let state = State::open(&state_db(&root), None).map_err(Fail::from_display("state"))?;
    let vault_id = state.vault_id().map_err(Fail::from_display("state"))?;
    let key_file = key_path(&vault_id)?;
    let key_created = if key_file.exists() {
        // VAULT_SECRET_KEY_FILE named a key that already exists; use it rather than replace it.
        SecretKey::load_file(&key_file).map_err(Fail::from_display("secret_key"))?;
        false
    } else {
        SecretKey::create_file(&key_file).map_err(Fail::from_display("secret_key"))?;
        true
    };
    state
        .set_password(&password)
        .map_err(Fail::from_display("state"))?;
    let mut out = json!({
        "vault": root,
        "vault_id": vault_id,
        "layout": store::LAYOUT_DIRS,
        "key_file": key_file,
        "key_created": key_created,
        "password_set": true,
    });
    if starter {
        out["starter"] = crate::starter::install(vault, state)?;
    }
    Ok(out)
}

/// `vault passwd`: replace the login password. [`State::set_password`] ends every browser session.
pub fn passwd(vault: &Path, password_stdin: bool) -> Result<Value, Fail> {
    let state = open_state(vault)?;
    let password = read_password(password_stdin)?;
    state
        .set_password(&password)
        .map_err(Fail::from_display("state"))?;
    Ok(json!({ "vault": vault, "password_set": true }))
}

/// `vault reindex`: rebuild `index.db` and the full-text index from the files, and report every
/// file that fails validation (D28). Fails when any file does.
pub fn reindex(vault: &Path) -> Result<Value, Fail> {
    open_state(vault)?;
    let vault = store::Vault::open(vault).map_err(Fail::from_display("store"))?;
    let mut index = index::Index::open(&vault).map_err(Fail::from_display("index"))?;
    let report = index.rebuild(&vault).map_err(Fail::from_display("index"))?;
    let failures: Vec<Value> = report
        .failures
        .iter()
        .map(|f| json!({ "path": f.path, "diagnostic": f.diagnostic }))
        .collect();
    let ok = failures.is_empty();
    let out = json!({
        "vault": vault.root(),
        "ok": ok,
        "indexed": report.indexed,
        "failures": failures,
    });
    if ok { Ok(out) } else { Err(Fail::body(out)) }
}

/// The password: one line of stdin with `--password-stdin`, or else asked for twice on the
/// terminal without echo.
fn read_password(from_stdin: bool) -> Result<String, Fail> {
    let password = if from_stdin {
        let mut raw = String::new();
        std::io::stdin()
            .read_to_string(&mut raw)
            .map_err(Fail::from_display("io"))?;
        let line = raw.strip_suffix('\n').unwrap_or(&raw);
        line.strip_suffix('\r').unwrap_or(line).to_owned()
    } else {
        if !std::io::stdin().is_terminal() {
            return Err(Fail::new(
                "usage",
                "stdin is not a terminal: pass --password-stdin and pipe the password in",
            ));
        }
        let first = prompt("Password: ")?;
        if prompt("Repeat password: ")? != first {
            return Err(Fail::new("usage", "the passwords do not match"));
        }
        first
    };
    if password.is_empty() {
        return Err(Fail::new("usage", "the password is empty"));
    }
    Ok(password)
}

/// Ask for a line on stderr and read it from the terminal with echo off.
fn prompt(label: &str) -> Result<String, Fail> {
    use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};
    let mut err = std::io::stderr();
    let _ = write!(err, "{label}");
    let _ = err.flush();
    let stdin = std::io::stdin();
    let saved = tcgetattr(&stdin).map_err(Fail::from_display("io"))?;
    let mut quiet = saved.clone();
    quiet.local_modes.remove(LocalModes::ECHO);
    tcsetattr(&stdin, OptionalActions::Now, &quiet).map_err(Fail::from_display("io"))?;
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    let _ = tcsetattr(&stdin, OptionalActions::Now, &saved);
    let _ = writeln!(err);
    read.map_err(Fail::from_display("io"))?;
    Ok(line.trim_end_matches(['\n', '\r']).to_owned())
}
