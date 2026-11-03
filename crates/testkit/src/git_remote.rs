//! Git remotes for the git-sync tests: a bare repo over `file://` ([`BareRepo`]), the same over
//! HTTP with basic auth through `git http-backend` ([`GitHttpServer`]), and a fake `ssh` that
//! runs the remote command locally ([`FakeSsh`]).
//!
//! ```no_run
//! let repo = testkit::BareRepo::new();
//! let url = repo.url(); // file:///tmp/.../repo.git
//!
//! let http = testkit::GitHttpServer::start("s3cret");
//! let url = http.url(); // http://127.0.0.1:<port>/repo.git
//!
//! let ssh = testkit::FakeSsh::new();
//! // Give the server VAULT_GITSYNC_SSH=<ssh.bin()>, and the remote ssh.url_for(repo.path()).
//! let calls = ssh.calls();
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use base64::Engine as _;
use tempfile::TempDir;

/// A bare repo, made with `git init --bare`, in a temporary folder. Dropping it deletes it.
pub struct BareRepo {
    _dir: TempDir,
    path: PathBuf,
}

impl BareRepo {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("testkit-git-")
            .tempdir()
            .expect("temp dir for a bare repo");
        let path = dir.path().join("repo.git");
        init_bare(&path);
        Self { _dir: dir, path }
    }

    /// The bare repo's folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `file://<path>`.
    pub fn url(&self) -> String {
        format!("file://{}", self.path.display())
    }

    /// Runs `git --git-dir <repo> <args>` and returns stdout, panicking when it fails. For
    /// inspecting the repo: `repo.git(&["ls-tree", "-r", "tv-..."])`.
    pub fn git(&self, args: &[&str]) -> String {
        git_in(&self.path, args)
    }

    /// The repo's branches, by name.
    pub fn branches(&self) -> Vec<String> {
        branches(&self.path)
    }
}

impl Default for BareRepo {
    fn default() -> Self {
        Self::new()
    }
}

/// A bare repo served over HTTP by `git http-backend` as CGI, on `127.0.0.1` and a free port.
/// Every request must carry basic auth whose password is the token; the username may be
/// anything. A request without it, or with another password, gets 401. Dropping it stops the
/// server and deletes the repo.
pub struct GitHttpServer {
    _dir: TempDir,
    repo: PathBuf,
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    refused: Arc<AtomicUsize>,
    served: Arc<AtomicUsize>,
}

impl GitHttpServer {
    /// Starts serving a new empty bare repo, requiring `token`.
    pub fn start(token: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("testkit-githttp-")
            .tempdir()
            .expect("temp dir for the http repo");
        let repo = dir.path().join("repo.git");
        init_bare(&repo);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the git http server");
        let addr = listener.local_addr().expect("listener address");
        let stop = Arc::new(AtomicBool::new(false));
        let refused = Arc::new(AtomicUsize::new(0));
        let served = Arc::new(AtomicUsize::new(0));
        let ctx = Arc::new(Ctx {
            root: dir.path().to_owned(),
            token: token.to_owned(),
            refused: refused.clone(),
            served: served.clone(),
        });
        let s = stop.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(conn) = conn else { continue };
                let ctx = ctx.clone();
                thread::spawn(move || {
                    let _ = serve(&ctx, conn);
                });
            }
        });
        Self {
            _dir: dir,
            repo,
            addr,
            token: token.to_owned(),
            stop,
            refused,
            served,
        }
    }

    /// `http://127.0.0.1:<port>/repo.git`, with no credentials in it.
    pub fn url(&self) -> String {
        format!("http://{}/repo.git", self.addr)
    }

    /// The bare repo's folder.
    pub fn repo(&self) -> &Path {
        &self.repo
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Requests answered 401.
    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::SeqCst)
    }

    /// Requests passed to `git http-backend`.
    pub fn served(&self) -> usize {
        self.served.load(Ordering::SeqCst)
    }

    /// Runs `git --git-dir <repo> <args>` and returns stdout, panicking when it fails.
    pub fn git(&self, args: &[&str]) -> String {
        git_in(&self.repo, args)
    }

    /// The repo's branches, by name.
    pub fn branches(&self) -> Vec<String> {
        branches(&self.repo)
    }
}

impl Drop for GitHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

struct Ctx {
    root: PathBuf,
    token: String,
    refused: Arc<AtomicUsize>,
    served: Arc<AtomicUsize>,
}

/// Answers one request on `conn`, then closes it.
fn serve(ctx: &Ctx, conn: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    if method.is_empty() {
        return Ok(());
    }
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let mut body = Vec::new();
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        read_chunked(&mut reader, &mut body)?;
    } else if let Some(len) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(len, 0);
        reader.read_exact(&mut body)?;
    }
    let mut out = conn;

    let authorised = header("authorization")
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|pair| pair.split_once(':').map(|(_, p)| p == ctx.token))
        .unwrap_or(false);
    if !authorised {
        ctx.refused.fetch_add(1, Ordering::SeqCst);
        out.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nwww-authenticate: Basic realm=\"git\"\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )?;
        return out.flush();
    }
    ctx.served.fetch_add(1, Ordering::SeqCst);

    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut cmd = Command::new("git");
    cmd.arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_PROJECT_ROOT", &ctx.root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("REMOTE_USER", "testkit")
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(v) = header("content-type") {
        cmd.env("CONTENT_TYPE", v);
    }
    if let Some(v) = header("content-encoding") {
        cmd.env("HTTP_CONTENT_ENCODING", v);
    }
    if let Some(v) = header("git-protocol") {
        cmd.env("GIT_PROTOCOL", v);
    }
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(&body);
    });
    let mut cgi = Vec::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_end(&mut cgi)?;
    let _ = writer.join();
    let _ = child.wait();

    // CGI output: headers, a blank line, then the body. `Status:` sets the status line.
    let split = find(&cgi, b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| find(&cgi, b"\n\n").map(|i| (i, 2)));
    let (head, rest) = match split {
        Some((i, n)) => (
            String::from_utf8_lossy(&cgi[..i]).into_owned(),
            &cgi[i + n..],
        ),
        None => (String::new(), &cgi[..]),
    };
    let mut status = "200 OK".to_owned();
    let mut reply = Vec::new();
    for h in head.lines() {
        let h = h.trim_end();
        if let Some(s) = h.strip_prefix("Status:") {
            status = s.trim().to_owned();
        } else if !h.is_empty() {
            reply.extend_from_slice(h.as_bytes());
            reply.extend_from_slice(b"\r\n");
        }
    }
    let mut response = format!("HTTP/1.1 {status}\r\n").into_bytes();
    response.extend_from_slice(&reply);
    response.extend_from_slice(
        format!(
            "content-length: {}\r\nconnection: close\r\n\r\n",
            rest.len()
        )
        .as_bytes(),
    );
    response.extend_from_slice(rest);
    out.write_all(&response)?;
    out.flush()
}

fn read_chunked(reader: &mut impl BufRead, body: &mut Vec<u8>) -> std::io::Result<()> {
    loop {
        let mut size = String::new();
        reader.read_line(&mut size)?;
        let size = size.trim().split(';').next().unwrap_or_default();
        let n = usize::from_str_radix(size, 16)
            .map_err(|_| std::io::Error::other(format!("bad chunk size {size:?}")))?;
        if n == 0 {
            // Trailers, up to the blank line.
            loop {
                let mut t = String::new();
                if reader.read_line(&mut t)? == 0 || t.trim().is_empty() {
                    return Ok(());
                }
            }
        }
        let start = body.len();
        body.resize(start + n, 0);
        reader.read_exact(&mut body[start..])?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The host key [`FakeSsh`] presents: what `accept-new` writes into `known_hosts`.
pub const FAKE_SSH_HOST_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFhScXDuFHnZpCUwNKcpYUi4XcUlC77oCdnt09UKWSu0";

/// The SHA256 fingerprint of [`FAKE_SSH_HOST_KEY`].
pub const FAKE_SSH_HOST_FINGERPRINT: &str = "SHA256:3oGTqCMsruXsZxfKDrhWoT8RwwwwJv068La9MZXqVEI";

/// The host [`FakeSsh::url_for`] puts in its URLs.
pub const FAKE_SSH_HOST: &str = "fakehost.test";

/// A fake `ssh` script in a temporary folder. Each call appends its argv to a log, checks that
/// `-i` names an existing file with mode 0600 and that `UserKnownHostsFile` is given, and checks
/// the host against that file as ssh would: a host not in it is added with
/// [`FAKE_SSH_HOST_KEY`] under `StrictHostKeyChecking=accept-new`, and refused otherwise. Then
/// it runs the remote command (its last argument) locally with `sh -c`, so the repo path is the
/// one in the URL.
pub struct FakeSsh {
    dir: TempDir,
}

const FAKE_SSH: &str = r#"#!/bin/sh
log='@LOG@'
{ for a in "$@"; do printf '%s\037' "$a"; done; printf '\n'; } >> "$log"
key= ; hosts= ; strict= ; host=
while [ $# -gt 1 ]; do
  case "$1" in
    -i) key=$2; shift 2 ;;
    -o)
      case "$2" in
        UserKnownHostsFile=*) hosts=${2#UserKnownHostsFile=} ;;
        StrictHostKeyChecking=*) strict=${2#StrictHostKeyChecking=} ;;
      esac
      shift 2 ;;
    -p|-l|-F) shift 2 ;;
    -*) shift ;;
    *) host=$1; shift ;;
  esac
done
if [ -z "$key" ] || [ ! -f "$key" ]; then echo "fake ssh: -i must name a key file" >&2; exit 255; fi
if [ "$(stat -c %a "$key")" != 600 ]; then echo "fake ssh: $key is not 0600" >&2; exit 255; fi
if [ -z "$hosts" ]; then echo "fake ssh: UserKnownHostsFile not given" >&2; exit 255; fi
name=${host#*@}
if ! grep -q "^$name " "$hosts" 2>/dev/null; then
  if [ "$strict" = accept-new ]; then
    printf '%s %s\n' "$name" '@HOSTKEY@' >> "$hosts"
  else
    echo "No $name host key is known and you have requested strict checking." >&2
    echo "Host key verification failed." >&2
    exit 255
  fi
elif ! grep -q "^$name @HOSTKEY@" "$hosts"; then
  echo "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!" >&2
  echo "Host key verification failed." >&2
  exit 255
fi
exec sh -c "$1"
"#;

impl FakeSsh {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("testkit-ssh-")
            .tempdir()
            .expect("temp dir for the fake ssh");
        let log = dir.path().join("calls.log");
        std::fs::write(&log, "").expect("create the fake ssh log");
        let script = FAKE_SSH
            .replace("@LOG@", &log.display().to_string())
            .replace("@HOSTKEY@", FAKE_SSH_HOST_KEY);
        let bin = dir.path().join("ssh");
        std::fs::write(&bin, script).expect("write the fake ssh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
                .expect("make the fake ssh executable");
        }
        Self { dir }
    }

    /// The script, for `VAULT_GITSYNC_SSH` or `Git::with_ssh_bin`.
    pub fn bin(&self) -> PathBuf {
        self.dir.path().join("ssh")
    }

    /// An `ssh://` URL for the bare repo at `repo`, on [`FAKE_SSH_HOST`].
    pub fn url_for(&self, repo: &Path) -> String {
        format!("ssh://git@{FAKE_SSH_HOST}{}", repo.display())
    }

    /// The argv of every call so far, oldest first.
    pub fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.dir.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                line.split('\u{1f}')
                    .filter(|a| !a.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .collect()
    }
}

impl Default for FakeSsh {
    fn default() -> Self {
        Self::new()
    }
}

fn init_bare(path: &Path) {
    let status = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .expect("run git init --bare");
    assert!(status.success(), "git init --bare {}", path.display());
}

fn git_in(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn branches(repo: &Path) -> Vec<String> {
    git_in(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}
