//! Seam S1: start `vault-server` on a fresh temporary vault and a random port. Seam S7's local
//! stand-in HTTP server is [`StandIn`].
//!
//! ```no_run
//! let server = testkit::TestServer::start();
//! let url = server.url("/api/health");
//! ```
//!
//! The server binary is found through `VAULT_SERVER_BIN`, or else in the cargo
//! target directory the test binary runs from. Cargo builds `vault-server`
//! before running the `server` crate's integration tests and before running
//! `cargo test --workspace`, so S1 tests normally live in `crates/server/tests`.
//!
//! Every endpoint but health and login needs credentials (D11). Before the server starts, the
//! testkit sets the login password to [`PASSWORD`] and creates an `admin` token named
//! [`ADMIN_TOKEN_NAME`], whose secret [`TestServer::admin_token`] returns. It also writes a secret
//! key file in a second temporary folder, outside the vault, and points the server at it through
//! `VAULT_SECRET_KEY_FILE` (OQ-S2), so secrets can be stored.
//!
//! A server gets 120 seconds to start. Set `TESTKIT_START_TIMEOUT_SECS` to change that.

mod standin;

pub use standin::{Reply, Request, StandIn};

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// How long a server gets to print its listen line and answer health. The full gate runs many
/// test binaries at once, so a start can take far longer than it does alone.
const DEFAULT_START_TIMEOUT: Duration = Duration::from_secs(120);

/// Overrides [`DEFAULT_START_TIMEOUT`], in whole seconds.
const START_TIMEOUT_VAR: &str = "TESTKIT_START_TIMEOUT_SECS";

fn start_timeout() -> Duration {
    parse_start_timeout(std::env::var(START_TIMEOUT_VAR).ok().as_deref())
}

fn parse_start_timeout(value: Option<&str>) -> Duration {
    let Some(value) = value else {
        return DEFAULT_START_TIMEOUT;
    };
    match value.trim().parse::<u64>() {
        Ok(secs) if secs > 0 => Duration::from_secs(secs),
        _ => panic!("{START_TIMEOUT_VAR} must be a whole number of seconds above 0, not {value:?}"),
    }
}

/// The login password of every test vault.
pub const PASSWORD: &str = "correct horse battery staple";

/// The name of the `admin` token every test vault starts with.
pub const ADMIN_TOKEN_NAME: &str = "admin";

/// A running `vault-server` on its own temporary vault. Dropping it kills the
/// server and deletes the vault.
pub struct TestServer {
    child: Child,
    addr: SocketAddr,
    vault: TempDir,
    /// Holds the secret key file, outside the vault.
    _config: TempDir,
    admin_token: String,
}

impl TestServer {
    /// Starts a server on a new empty vault and waits until `/api/health` answers.
    pub fn start() -> Self {
        Self::start_with_binary(&server_binary())
    }

    /// Starts a server on a new vault after `seed` has written files into the vault folder, such
    /// as the type definitions a test needs.
    pub fn start_seeded(seed: impl FnOnce(&Path)) -> Self {
        Self::start_inner(&server_binary(), seed, &[])
    }

    /// Like [`TestServer::start_seeded`], passing `args` to `vault-server` as well, such as
    /// `--web <dir>`.
    pub fn start_with_args(seed: impl FnOnce(&Path), args: &[&std::ffi::OsStr]) -> Self {
        Self::start_inner(&server_binary(), seed, args)
    }

    /// Like [`TestServer::start`], with an explicit path to the `vault-server` binary.
    pub fn start_with_binary(binary: &Path) -> Self {
        Self::start_inner(binary, |_| {}, &[])
    }

    fn start_inner(binary: &Path, seed: impl FnOnce(&Path), args: &[&std::ffi::OsStr]) -> Self {
        let vault = tempfile::Builder::new()
            .prefix("thevault-test-")
            .tempdir()
            .expect("create temporary vault directory");
        seed(vault.path());
        let admin_token = seed_credentials(vault.path());
        let config = tempfile::Builder::new()
            .prefix("thevault-config-")
            .tempdir()
            .expect("create temporary config directory");
        let key = config.path().join("vault.key");
        state::SecretKey::create_file(&key).expect("write the secret key file");
        let mut command = Command::new(binary);
        command.env("VAULT_SECRET_KEY_FILE", &key).args(args);
        Self::launch(command, binary, vault, config, admin_token)
    }

    /// Serves a vault folder that already holds its `state.db`, such as one `vault init` made or
    /// one restored from a backup, setting nothing up first. `admin_token` is whatever token secret
    /// that vault already accepts, or empty. The server sees `env` and no `VAULT_SECRET_KEY_FILE`
    /// unless `env` names one, so it finds its key the way a real server does (OQ-S2).
    pub fn start_existing(vault: TempDir, admin_token: &str, env: &[(&str, &Path)]) -> Self {
        let binary = server_binary();
        let config = tempfile::Builder::new()
            .prefix("thevault-config-")
            .tempdir()
            .expect("create temporary config directory");
        let mut command = Command::new(&binary);
        command.env_remove("VAULT_SECRET_KEY_FILE");
        for (name, value) in env {
            command.env(name, value);
        }
        Self::launch(command, &binary, vault, config, admin_token.to_owned())
    }

    fn launch(
        mut command: Command,
        binary: &Path,
        vault: TempDir,
        config: TempDir,
        admin_token: String,
    ) -> Self {
        let mut child = command
            .arg("--vault")
            .arg(vault.path())
            .arg("--listen")
            .arg("127.0.0.1:0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("start {}: {e}", binary.display()));

        let addr = match read_listen_line(&mut child) {
            Ok(addr) => addr,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("vault-server did not report its address: {e}");
            }
        };

        let server = TestServer {
            child,
            addr,
            vault,
            _config: config,
            admin_token,
        };
        server.wait_healthy();
        server
    }

    /// The base URL, such as `http://127.0.0.1:41234`, with no trailing slash.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The base URL joined with `path`, which starts with `/`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url())
    }

    /// The address the server is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The secret of the vault's `admin` token, to send as `Authorization: Bearer <secret>`.
    pub fn admin_token(&self) -> &str {
        &self.admin_token
    }

    /// The temporary vault folder the server is serving.
    pub fn vault_path(&self) -> &Path {
        self.vault.path()
    }

    fn wait_healthy(&self) {
        let deadline = Instant::now() + start_timeout();
        loop {
            match http_get_status(self.addr, "/api/health") {
                Ok(200) => return,
                Ok(_) | Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25))
                }
                Ok(status) => panic!("/api/health answered {status}"),
                Err(e) => panic!("/api/health did not answer: {e}"),
            }
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sets the login password to [`PASSWORD`] and creates the admin token in the vault's
/// `state.db`, returning the token's secret. The server opens the same file when it starts.
pub fn seed_credentials(vault: &Path) -> String {
    let state = state::State::open(&vault.join(".vault/state.db"), None).expect("open state.db");
    state.set_password(PASSWORD).expect("set the test password");
    let (_, secret) = state
        .create_token(ADMIN_TOKEN_NAME, state::Scope::Admin)
        .expect("create the admin token");
    secret
}

/// Reads the JSON line the server prints on start, then keeps draining stdout
/// on a background thread so the server never blocks on a full pipe.
fn read_listen_line(child: &mut Child) -> Result<SocketAddr, String> {
    let stdout = child.stdout.take().ok_or("no stdout pipe")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let first = reader.read_line(&mut line).map(|_| line);
        let _ = tx.send(first);
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });

    let line = rx
        .recv_timeout(start_timeout())
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| format!("{e} in {line:?}"))?;
    value["listen"]
        .as_str()
        .ok_or_else(|| format!("no `listen` in {line:?}"))?
        .parse()
        .map_err(|e| format!("{e} in {line:?}"))
}

/// A bare HTTP/1.1 GET that returns the status code. Used only to wait for
/// health, so the testkit needs no async runtime or HTTP client.
fn http_get_status(addr: SocketAddr, path: &str) -> std::io::Result<u16> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::other(format!("bad response: {response:?}")))
}

/// Finds the `vault-server` binary: `VAULT_SERVER_BIN`, or else the cargo
/// target directory that holds the running test binary.
pub fn server_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("VAULT_SERVER_BIN") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("current test binary path");
    // Test binaries run from target/<profile>/deps/.
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("test binary inside a cargo target directory");
    let binary = profile_dir.join(format!("vault-server{}", std::env::consts::EXE_SUFFIX));
    assert!(
        binary.exists(),
        "{} does not exist. Run `cargo build -p server` first, or set VAULT_SERVER_BIN.",
        binary.display()
    );
    binary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_timeout_defaults_to_120_seconds() {
        assert_eq!(parse_start_timeout(None), Duration::from_secs(120));
    }

    #[test]
    fn the_start_timeout_reads_whole_seconds_from_the_variable() {
        assert_eq!(parse_start_timeout(Some(" 300 ")), Duration::from_secs(300));
    }

    #[test]
    #[should_panic(expected = "TESTKIT_START_TIMEOUT_SECS")]
    fn a_start_timeout_that_is_not_a_number_is_refused() {
        parse_start_timeout(Some("soon"));
    }

    #[test]
    #[should_panic(expected = "TESTKIT_START_TIMEOUT_SECS")]
    fn a_zero_start_timeout_is_refused() {
        parse_start_timeout(Some("0"));
    }
}
