//! The `git` CLI runner (git-sync D22, D24, D19).
//!
//! [`Git`] runs `git` against one bare repo with a scrubbed environment: nothing inherited but
//! `PATH`, `HOME` pointing at a private temporary folder, no system or global config, and
//! `GIT_TERMINAL_PROMPT=0`. No credential is ever passed in argv or the URL:
//!
//! - [`Auth::Https`] writes an askpass script, readable only by its owner, that echoes the
//!   username and token from variables set only on the git child, and passes it as
//!   `GIT_ASKPASS` (D24).
//! - [`Auth::Ssh`] writes the private key and a `known_hosts` file to 0600 files, the latter
//!   preloaded with [`GITHUB_KNOWN_HOSTS`] and the pinned lines, and sets `GIT_SSH_COMMAND`.
//!   `StrictHostKeyChecking` is `accept-new` only while nothing is pinned; after such a run,
//!   [`Git::learned_host_line`] reads back the line ssh added, for the caller to pin (D19).
//!
//! The temporary files are deleted when the runner drops. A command that runs past the timeout
//! is killed with its whole process group. The stderr in a returned [`Error::Git`] has the token
//! and every temporary path redacted.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use crate::{Error, Result};

/// GitHub's published SSH host keys (from `https://api.github.com/meta`, as listed in GitHub's
/// docs), preloaded into every `known_hosts` file.
pub const GITHUB_KNOWN_HOSTS: &str = "\
github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl
github.com ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=
github.com ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQCj7ndNxQowgcQnjshcLrqPEiiphnt+VTTvDP6mHBL9j1aNUkY4Ue1gvwnGLVlOhGeYrnZaMgRK6+PKCUXaDbC7qtbW8gIkhL7aGCsOr/C56SJMy/BCZfxd1nWzAOxSDPgVsmerOBYfNqltV9/hWCqBywINIR+5dIg6JTJ72pcEpEjcYgXkE2YEFXV1JHnsKgbLWNlhScqb2UmyRkQyytRLtL+38TGxkxCflmO+5Z8CSSNY7GidjMIZ7Q4zMjA2n1nGrlTDkzwDCsw+wqFPGQA179cnfGWOWRVruj16z6XyvxvjJwbz0wQZ75XK5tKSb7FNyeIEs4TT4jk+S4dhPeAUC5y+bDYirYgM4GC7uEnztnZyaVWQ7B381AK4Qdrwt51ZqExKbQpTUNn+EjqoTwvqNj4kqx5QUCI0ThS/YkOxJCXmPUWZbhjpCg56i+2aB6CmK2JGhn57K5mj0MNdBXA4/WnwH6XoPWJzK5Nyu2zB3nAZp+S5hpQs+p1vN1/wsjk=
";

/// The HTTPS username when none is given: what GitHub expects with a token (D24).
pub const DEFAULT_HTTPS_USERNAME: &str = "x-access-token";

/// How long one git command may run before it is killed.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The child-only variables the askpass script reads.
pub const TOKEN_VAR: &str = "TVGS_GIT_TOKEN";
pub const USERNAME_VAR: &str = "TVGS_GIT_USERNAME";

/// What stands in for a redacted secret or path in returned stderr.
pub const REDACTED: &str = "[redacted]";

/// The refspec a round fetches: every instance branch, into `refs/remotes/origin/`.
pub const INSTANCE_REFSPEC: &str = "+refs/heads/tv-*:refs/remotes/origin/tv-*";

const ASKPASS: &str = "#!/bin/sh\n\
case \"$1\" in\n\
  [Uu]sername*) printf '%s\\n' \"$TVGS_GIT_USERNAME\" ;;\n\
  *) printf '%s\\n' \"$TVGS_GIT_TOKEN\" ;;\n\
esac\n";

/// How the runner authenticates to the remote.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// No credentials, as for a `file://` remote.
    None,
    /// A token over HTTPS. An empty username means [`DEFAULT_HTTPS_USERNAME`].
    Https { username: String, token: String },
    /// An SSH key. `known_hosts` holds the pinned host lines, empty while none is pinned.
    Ssh {
        private_key: String,
        known_hosts: String,
    },
}

impl Auth {
    /// HTTPS with `token` and the default username.
    pub fn https(token: impl Into<String>) -> Self {
        Auth::Https {
            username: DEFAULT_HTTPS_USERNAME.into(),
            token: token.into(),
        }
    }
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::None => f.write_str("None"),
            Auth::Https { username, .. } => f
                .debug_struct("Https")
                .field("username", username)
                .finish_non_exhaustive(),
            Auth::Ssh { known_hosts, .. } => f
                .debug_struct("Ssh")
                .field("pinned", &!known_hosts.trim().is_empty())
                .finish_non_exhaustive(),
        }
    }
}

/// One entry of `git ls-tree -r -l`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: String,
    /// `blob`, `tree` or `commit`.
    pub kind: String,
    pub oid: String,
    /// The blob's size; `None` for anything else.
    pub size: Option<u64>,
    pub path: String,
}

/// What a command returned, whether it succeeded or not.
#[derive(Debug)]
struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    /// Already redacted.
    stderr: String,
}

type Feed<'a> = Box<dyn FnOnce(&mut dyn Write) -> std::io::Result<()> + Send + 'a>;

/// The runner. See the module docs.
pub struct Git {
    bin: PathBuf,
    ssh_bin: PathBuf,
    dir: PathBuf,
    auth: Auth,
    timeout: Duration,
    /// `HOME`, the askpass script, the key, `known_hosts` and scratch indexes. Deleted on drop.
    tmp: TempDir,
    askpass: Option<PathBuf>,
    key_file: Option<PathBuf>,
    known_hosts_file: Option<PathBuf>,
    /// The `known_hosts` file as written, to tell a learned line from the preloaded ones.
    known_hosts_initial: String,
}

impl std::fmt::Debug for Git {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Git")
            .field("bin", &self.bin)
            .field("ssh_bin", &self.ssh_bin)
            .field("dir", &self.dir)
            .field("auth", &self.auth)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Git {
    /// A runner for the bare repo at `dir` (which need not exist yet), with `git` and `ssh` from
    /// `PATH` and [`DEFAULT_TIMEOUT`]. Writes the credential files `auth` needs.
    pub fn new(dir: impl Into<PathBuf>, auth: Auth) -> Result<Self> {
        let tmp = tempfile::Builder::new().prefix("tv-gitsync-").tempdir()?;
        let mut git = Self {
            bin: "git".into(),
            ssh_bin: "ssh".into(),
            dir: dir.into(),
            auth,
            timeout: DEFAULT_TIMEOUT,
            tmp,
            askpass: None,
            key_file: None,
            known_hosts_file: None,
            known_hosts_initial: String::new(),
        };
        match &git.auth {
            Auth::None => {}
            Auth::Https { .. } => {
                // Owner-only, and executable since git runs it directly.
                let path = git.tmp.path().join("askpass");
                write_private(&path, ASKPASS.as_bytes(), 0o700)?;
                git.askpass = Some(path);
            }
            Auth::Ssh {
                private_key,
                known_hosts,
            } => {
                let key = git.tmp.path().join("id");
                let mut text = private_key.trim().to_owned();
                text.push('\n');
                write_private(&key, text.as_bytes(), 0o600)?;
                let mut hosts = GITHUB_KNOWN_HOSTS.to_owned();
                for line in known_hosts.lines().filter(|l| !l.trim().is_empty()) {
                    hosts.push_str(line.trim());
                    hosts.push('\n');
                }
                let file = git.tmp.path().join("known_hosts");
                write_private(&file, hosts.as_bytes(), 0o600)?;
                git.key_file = Some(key);
                git.known_hosts_file = Some(file);
                git.known_hosts_initial = hosts;
            }
        }
        Ok(git)
    }

    /// Runs `bin` instead of `git` from `PATH`.
    pub fn with_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.bin = bin.into();
        self
    }

    /// Runs `ssh_bin` instead of `ssh` from `PATH`: the server's `VAULT_GITSYNC_SSH` test seam.
    pub fn with_ssh_bin(mut self, ssh_bin: impl Into<PathBuf>) -> Self {
        self.ssh_bin = ssh_bin.into();
        self
    }

    /// Kills a command that runs longer than `timeout`.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The bare repo's folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn auth(&self) -> &Auth {
        &self.auth
    }

    /// The `StrictHostKeyChecking` value SSH runs with: `accept-new` while no host key is
    /// pinned, `yes` once one is. `None` for other auth.
    pub fn strict_host_key_checking(&self) -> Option<&'static str> {
        match &self.auth {
            Auth::Ssh { known_hosts, .. } if known_hosts.trim().is_empty() => Some("accept-new"),
            Auth::Ssh { .. } => Some("yes"),
            _ => None,
        }
    }

    /// The host lines ssh added to `known_hosts` under `accept-new`, joined by newlines, for the
    /// caller to pin once the run succeeded. `None` when it added none.
    pub fn learned_host_line(&self) -> Option<String> {
        let file = self.known_hosts_file.as_ref()?;
        let now = std::fs::read_to_string(file).ok()?;
        let before: std::collections::HashSet<&str> =
            self.known_hosts_initial.lines().map(str::trim).collect();
        let learned: Vec<&str> = now
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !before.contains(l))
            .collect();
        (!learned.is_empty()).then(|| learned.join("\n"))
    }

    /// The command that runs `git --git-dir <dir> <args>`, with the scrubbed environment and the
    /// auth variables. The tests inspect it; the operations below run it.
    pub fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut cmd = self.bare_command();
        cmd.arg("--git-dir").arg(&self.dir);
        cmd.args(args);
        cmd
    }

    /// `git` with no `--git-dir`, for `init`.
    fn bare_command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env_clear();
        cmd.env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()),
        );
        cmd.env("HOME", self.tmp.path());
        cmd.env("TMPDIR", self.tmp.path());
        cmd.env("LC_ALL", "C");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("GIT_CONFIG_NOSYSTEM", "1");
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
        for who in ["AUTHOR", "COMMITTER"] {
            cmd.env(format!("GIT_{who}_NAME"), "TheVault gitsync");
            cmd.env(format!("GIT_{who}_EMAIL"), "gitsync@thevault.invalid");
        }
        match &self.auth {
            Auth::None => {}
            Auth::Https { username, token } => {
                let username = if username.is_empty() {
                    DEFAULT_HTTPS_USERNAME
                } else {
                    username
                };
                if let Some(askpass) = &self.askpass {
                    cmd.env("GIT_ASKPASS", askpass);
                }
                cmd.env(USERNAME_VAR, username);
                cmd.env(TOKEN_VAR, token);
            }
            Auth::Ssh { .. } => {
                cmd.env("GIT_SSH_COMMAND", self.ssh_command());
                // The OpenSSH options, whatever the ssh binary is called.
                cmd.env("GIT_SSH_VARIANT", "ssh");
            }
        }
        cmd
    }

    fn ssh_command(&self) -> String {
        let key = self.key_file.as_deref().unwrap_or(Path::new(""));
        let hosts = self.known_hosts_file.as_deref().unwrap_or(Path::new(""));
        format!(
            "{} -i {} -o IdentitiesOnly=yes -o {} -o StrictHostKeyChecking={}",
            shell_quote(&self.ssh_bin.to_string_lossy()),
            shell_quote(&key.to_string_lossy()),
            shell_quote(&format!("UserKnownHostsFile={}", hosts.display())),
            self.strict_host_key_checking().unwrap_or("yes"),
        )
    }

    /// `stderr` with the token and every temporary path replaced by [`REDACTED`].
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        if let Auth::Https { token, .. } = &self.auth
            && !token.is_empty()
        {
            out = out.replace(token.as_str(), REDACTED);
        }
        let tmp = self.tmp.path().to_string_lossy().into_owned();
        out = out.replace(&tmp, REDACTED);
        if let Ok(real) = self.tmp.path().canonicalize() {
            out = out.replace(real.to_string_lossy().as_ref(), REDACTED);
        }
        out
    }

    /// Runs `cmd`, feeding it `feed` on stdin, and kills its process group past the timeout.
    fn exec(&self, mut cmd: Command, feed: Option<Feed<'_>>, what: &str) -> Result<Output> {
        cmd.stdin(if feed.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, so a timeout kills ssh and the remote helpers too.
        .process_group(0);
        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let deadline = Instant::now() + self.timeout;
        let (status, out, err) = std::thread::scope(|s| {
            if let (Some(feed), Some(mut pipe)) = (feed, stdin.take()) {
                s.spawn(move || {
                    // A child that exits early closes the pipe; its status tells why.
                    let _ = feed(&mut pipe);
                });
            }
            let out = s.spawn(move || {
                let mut buf = Vec::new();
                let _ = stdout.read_to_end(&mut buf);
                buf
            });
            let err = s.spawn(move || {
                let mut buf = Vec::new();
                let _ = stderr.read_to_end(&mut buf);
                buf
            });
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if Instant::now() >= deadline => {
                        kill_group(child.id());
                        let _ = child.wait();
                        break Err(Error::Timeout {
                            command: what.to_owned(),
                            secs: self.timeout.as_secs(),
                        });
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    Err(e) => {
                        kill_group(child.id());
                        let _ = child.wait();
                        break Err(e.into());
                    }
                }
            };
            if status.is_ok() {
                // git has exited; anything it left behind in its group would hold the pipes.
                kill_group(child.id());
            }
            (
                status,
                out.join().unwrap_or_default(),
                err.join().unwrap_or_default(),
            )
        });
        Ok(Output {
            status: status?,
            stdout: out,
            stderr: self.redact(String::from_utf8_lossy(&err).trim()),
        })
    }

    /// Runs `git <args>` and returns stdout, or [`Error::Git`] when it exits non-zero.
    fn run(&self, args: &[&str], feed: Option<Feed<'_>>) -> Result<Vec<u8>> {
        self.run_env(args, &[], feed)
    }

    fn run_env(
        &self,
        args: &[&str],
        env: &[(&str, &OsStr)],
        feed: Option<Feed<'_>>,
    ) -> Result<Vec<u8>> {
        let what = args.first().copied().unwrap_or("");
        let mut cmd = self.command(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = self.exec(cmd, feed, what)?;
        if !out.status.success() {
            return Err(self.failed(what, &out));
        }
        Ok(out.stdout)
    }

    fn failed(&self, what: &str, out: &Output) -> Error {
        Error::Git {
            command: what.to_owned(),
            status: out
                .status
                .code()
                .map_or_else(|| "killed".to_owned(), |c| format!("exit {c}")),
            stderr: out.stderr.clone(),
        }
    }

    /// Makes the bare repo at [`Git::dir`] if it is missing, and points its `origin` at
    /// `remote_url`.
    pub fn ensure_bare_clone(&self, remote_url: &str) -> Result<()> {
        if !self.dir.join("HEAD").is_file() {
            std::fs::create_dir_all(&self.dir)?;
            let mut cmd = self.bare_command();
            cmd.args(["init", "--bare", "--quiet"]).arg(&self.dir);
            let out = self.exec(cmd, None, "init")?;
            if !out.status.success() {
                return Err(self.failed("init", &out));
            }
        }
        let current = self.exec(
            self.command(&["config", "--get", "remote.origin.url"]),
            None,
            "config",
        )?;
        if !current.status.success() {
            self.run(&["remote", "add", "origin", remote_url], None)?;
        } else if String::from_utf8_lossy(&current.stdout).trim() != remote_url {
            self.run(&["remote", "set-url", "origin", remote_url], None)?;
        }
        Ok(())
    }

    /// Fetches every `tv-*` branch of `origin` into `refs/remotes/origin/`, dropping the ones
    /// gone from the remote. An empty remote is not an error.
    pub fn fetch_instance_branches(&self) -> Result<()> {
        self.run(
            &["fetch", "--prune", "--no-tags", "origin", INSTANCE_REFSPEC],
            None,
        )?;
        Ok(())
    }

    /// The `tv-*` branches the last fetch brought in, as (branch, commit) pairs, by name.
    pub fn fetched_branches(&self) -> Result<Vec<(String, String)>> {
        let out = self.run(
            &[
                "for-each-ref",
                "--format=%(refname:lstrip=3) %(objectname)",
                "refs/remotes/origin/",
            ],
            None,
        )?;
        Ok(pairs(&out)
            .filter(|(name, _)| name.starts_with(crate::layout::BRANCH_PREFIX))
            .collect())
    }

    /// Every branch on the remote as (branch, commit) pairs, asked of the remote itself.
    pub fn list_remote_branches(&self) -> Result<Vec<(String, String)>> {
        let out = self.run(&["ls-remote", "--heads", "origin"], None)?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|line| {
                let (oid, name) = line.split_once('\t')?;
                Some((name.strip_prefix("refs/heads/")?.to_owned(), oid.to_owned()))
            })
            .collect())
    }

    /// Every blob under `treeish`, recursively.
    pub fn read_tree(&self, treeish: &str) -> Result<Vec<TreeEntry>> {
        let out = self.run(&["ls-tree", "-r", "-l", "-z", treeish], None)?;
        let mut entries = Vec::new();
        for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            let Some((meta, path)) = record.split_once('\t') else {
                continue;
            };
            let mut fields = meta.split_whitespace();
            let (Some(mode), Some(kind), Some(oid), size) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            entries.push(TreeEntry {
                mode: mode.to_owned(),
                kind: kind.to_owned(),
                oid: oid.to_owned(),
                size: size.and_then(|s| s.parse().ok()),
                path: path.to_owned(),
            });
        }
        Ok(entries)
    }

    /// The bytes of blob `oid`.
    pub fn cat_blob(&self, oid: &str) -> Result<Vec<u8>> {
        self.run(&["cat-file", "blob", oid], None)
    }

    /// The bytes of each blob in `oids`, in order, through one `git cat-file --batch`.
    pub fn cat_blobs(&self, oids: &[&str]) -> Result<Vec<Vec<u8>>> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let list: String = oids.iter().map(|o| format!("{o}\n")).collect();
        let out = self.run(
            &["cat-file", "--batch"],
            Some(Box::new(move |w| w.write_all(list.as_bytes()))),
        )?;
        let mut blobs = Vec::with_capacity(oids.len());
        let mut rest = out.as_slice();
        for oid in oids {
            let nl = rest
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| Error::Malformed("cat-file --batch output ended early".into()))?;
            let head = String::from_utf8_lossy(&rest[..nl]).into_owned();
            let mut fields = head.split_whitespace();
            let (Some(_), Some(kind), Some(size)) = (fields.next(), fields.next(), fields.next())
            else {
                return Err(Error::Malformed(format!("object {oid}: {head}")));
            };
            if kind != "blob" {
                return Err(Error::Malformed(format!("object {oid} is a {kind}")));
            }
            let size: usize = size
                .parse()
                .map_err(|_| Error::Malformed(format!("object {oid}: {head}")))?;
            let start = nl + 1;
            if rest.len() < start + size + 1 {
                return Err(Error::Malformed(format!("object {oid} is cut short")));
            }
            blobs.push(rest[start..start + size].to_vec());
            rest = &rest[start + size + 1..];
        }
        Ok(blobs)
    }

    /// Writes every file in `files` (path from the tree root to bytes) as a blob, and the trees
    /// above them, and returns the root tree's id. Blobs go in through one `git fast-import`,
    /// and the trees through a scratch index.
    pub fn write_tree(&self, files: &BTreeMap<String, Vec<u8>>) -> Result<String> {
        let scratch = tempfile::Builder::new()
            .prefix("tree-")
            .tempdir_in(self.tmp.path())?;
        let marks = scratch.path().join("marks");
        let index = scratch.path().join("index");
        let marks_arg = format!("--export-marks={}", marks.display());
        self.run(
            &["fast-import", "--quiet", "--done", &marks_arg],
            Some(Box::new(|w| {
                for (n, bytes) in files.values().enumerate() {
                    write!(w, "blob\nmark :{}\ndata {}\n", n + 1, bytes.len())?;
                    w.write_all(bytes)?;
                    w.write_all(b"\n")?;
                }
                w.write_all(b"done\n")
            })),
        )?;
        let mut oids = vec![String::new(); files.len()];
        for line in std::fs::read_to_string(&marks).unwrap_or_default().lines() {
            if let Some((mark, oid)) = line.split_once(' ')
                && let Some(n) = mark.strip_prefix(':').and_then(|n| n.parse::<usize>().ok())
                && (1..=oids.len()).contains(&n)
            {
                oids[n - 1] = oid.to_owned();
            }
        }
        if oids.iter().any(String::is_empty) {
            return Err(Error::Malformed("fast-import left a blob unmarked".into()));
        }
        let mut info = Vec::new();
        for (path, oid) in files.keys().zip(&oids) {
            info.extend_from_slice(format!("100644 blob {oid}\t{path}").as_bytes());
            info.push(0);
        }
        let env: [(&str, &OsStr); 1] = [("GIT_INDEX_FILE", index.as_os_str())];
        self.run_env(
            &["update-index", "--add", "-z", "--index-info"],
            &env,
            Some(Box::new(move |w| w.write_all(&info))),
        )?;
        let out = self.run_env(&["write-tree"], &env, None)?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// A commit of `tree` with no parent, and its id (D11).
    pub fn commit_tree(&self, tree: &str, message: &str) -> Result<String> {
        let out = self.run(&["commit-tree", tree, "-m", message], None)?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// Force-pushes `commit` to `refs/heads/<branch>` on `origin` (D11).
    pub fn force_push(&self, commit: &str, branch: &str) -> Result<()> {
        let refspec = format!("{commit}:refs/heads/{branch}");
        self.run(&["push", "--force", "--quiet", "origin", &refspec], None)?;
        Ok(())
    }

    /// The tree of `rev`, such as `refs/remotes/origin/tv-<name>`, or `None` when it does not
    /// exist.
    pub fn tree_of(&self, rev: &str) -> Result<Option<String>> {
        let spec = format!("{rev}^{{tree}}");
        let out = self.exec(
            self.command(&["rev-parse", "--verify", "--quiet", &spec]),
            None,
            "rev-parse",
        )?;
        if !out.status.success() {
            if out.stderr.is_empty() {
                return Ok(None);
            }
            return Err(self.failed("rev-parse", &out));
        }
        let tree = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok((!tree.is_empty()).then_some(tree))
    }
}

fn pairs(out: &[u8]) -> impl Iterator<Item = (String, String)> + '_ {
    std::str::from_utf8(out)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (name, oid) = line.split_once(' ')?;
            Some((name.to_owned(), oid.to_owned()))
        })
}

fn write_private(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn kill_group(pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

/// `text` in single quotes for `sh`.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}
