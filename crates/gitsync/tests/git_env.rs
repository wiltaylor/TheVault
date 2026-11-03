//! How the runner builds a git command: the scrubbed environment, the askpass script, the ssh
//! command, and redaction.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use gitsync::git::{REDACTED, TOKEN_VAR, USERNAME_VAR};
use gitsync::{Auth, Error, Git};

const TOKEN: &str = "ghp_secretTOKEN123";

fn envs(cmd: &Command) -> HashMap<String, String> {
    cmd.get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().into_owned(),
                v?.to_string_lossy().into_owned(),
            ))
        })
        .collect()
}

fn args(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn mode(path: &str) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn https_auth_uses_an_owner_only_askpass_script_and_no_argv_token() {
    let repo = tempfile::tempdir().unwrap();
    let git = Git::new(repo.path().join("r.git"), Auth::https(TOKEN)).unwrap();
    let cmd = git.command(&["fetch", "origin"]);
    let env = envs(&cmd);

    assert!(args(&cmd).iter().all(|a| !a.contains(TOKEN)));
    assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
    assert_eq!(env[TOKEN_VAR], TOKEN);
    assert_eq!(env[USERNAME_VAR], "x-access-token");
    // The token is in exactly one variable, the one only the child gets.
    let holders: Vec<&String> = env
        .iter()
        .filter(|(_, v)| v.contains(TOKEN))
        .map(|(k, _)| k)
        .collect();
    assert_eq!(holders, [TOKEN_VAR]);

    let askpass = &env["GIT_ASKPASS"];
    assert_eq!(mode(askpass), 0o700);
    let script = std::fs::read_to_string(askpass).unwrap();
    assert!(!script.contains(TOKEN), "{script}");

    // The script answers git's two prompts from the child's variables.
    let ask = |prompt: &str| {
        let out = Command::new(askpass)
            .arg(prompt)
            .env_clear()
            .env(TOKEN_VAR, TOKEN)
            .env(USERNAME_VAR, "x-access-token")
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(
        ask("Username for 'https://github.com': "),
        "x-access-token\n"
    );
    assert_eq!(
        ask("Password for 'https://x@github.com': "),
        format!("{TOKEN}\n")
    );

    // The temporary files go with the runner.
    let askpass = PathBuf::from(askpass);
    drop(git);
    assert!(!askpass.exists());
}

#[test]
fn a_given_https_username_is_used() {
    let git = Git::new(
        "/nonexistent",
        Auth::Https {
            username: "wil".into(),
            token: TOKEN.into(),
        },
    )
    .unwrap();
    assert_eq!(envs(&git.command(&["fetch"]))[USERNAME_VAR], "wil");
    let git = Git::new(
        "/nonexistent",
        Auth::Https {
            username: String::new(),
            token: TOKEN.into(),
        },
    )
    .unwrap();
    assert_eq!(
        envs(&git.command(&["fetch"]))[USERNAME_VAR],
        "x-access-token"
    );
}

#[test]
fn the_environment_is_scrubbed() {
    // A stand-in git that records the environment it was given.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("env.txt");
    let bin = dir.path().join("git");
    std::fs::write(&bin, format!("#!/bin/sh\nenv > '{}'\n", out.display())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let git = Git::new(dir.path().join("r.git"), Auth::https(TOKEN))
        .unwrap()
        .with_bin(&bin);
    git.fetch_instance_branches().unwrap();
    let seen = std::fs::read_to_string(&out).unwrap();
    let names: Vec<&str> = seen
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, _)| k))
        .collect();
    let allowed = [
        "PATH",
        "HOME",
        "TMPDIR",
        "LC_ALL",
        "PWD",
        "SHLVL",
        "_",
        "GIT_TERMINAL_PROMPT",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_ASKPASS",
        TOKEN_VAR,
        USERNAME_VAR,
    ];
    for name in &names {
        assert!(
            allowed.contains(name),
            "{name} leaked into git's environment"
        );
    }
    assert!(names.contains(&"GIT_TERMINAL_PROMPT"));
    let home = seen.lines().find_map(|l| l.strip_prefix("HOME=")).unwrap();
    assert_ne!(Some(home.to_owned()), std::env::var("HOME").ok());
}

#[test]
fn ssh_auth_writes_0600_files_and_the_ssh_command() {
    let git =
        Git::new(
            "/nonexistent",
            Auth::Ssh {
                private_key:
                    "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
                        .into(),
                known_hosts: String::new(),
            },
        )
        .unwrap()
        .with_ssh_bin("/opt/my ssh");
    assert_eq!(git.strict_host_key_checking(), Some("accept-new"));
    let env = envs(&git.command(&["fetch"]));
    let ssh = &env["GIT_SSH_COMMAND"];
    assert!(ssh.starts_with("'/opt/my ssh' -i '"), "{ssh}");
    assert!(ssh.contains(" -o IdentitiesOnly=yes "), "{ssh}");
    assert!(
        ssh.ends_with(" -o StrictHostKeyChecking=accept-new"),
        "{ssh}"
    );

    let key = ssh.split('\'').nth(3).unwrap();
    assert_eq!(mode(key), 0o600);
    assert!(std::fs::read_to_string(key).unwrap().contains("abc"));
    let hosts = ssh
        .split("UserKnownHostsFile=")
        .nth(1)
        .unwrap()
        .split('\'')
        .next()
        .unwrap();
    assert_eq!(mode(hosts), 0o600);
    let text = std::fs::read_to_string(hosts).unwrap();
    assert!(text.starts_with(gitsync::GITHUB_KNOWN_HOSTS));
    assert_eq!(git.learned_host_line(), None);

    // A learned line is read back.
    std::fs::write(hosts, format!("{text}example.com ssh-ed25519 AAAAxyz\n")).unwrap();
    assert_eq!(
        git.learned_host_line().as_deref(),
        Some("example.com ssh-ed25519 AAAAxyz")
    );
}

#[test]
fn a_pinned_host_key_means_strict_checking() {
    let pinned = "example.com ssh-ed25519 AAAAxyz";
    let git = Git::new(
        "/nonexistent",
        Auth::Ssh {
            private_key: "k".into(),
            known_hosts: format!("{pinned}\n"),
        },
    )
    .unwrap();
    assert_eq!(git.strict_host_key_checking(), Some("yes"));
    let env = envs(&git.command(&["fetch"]));
    assert!(env["GIT_SSH_COMMAND"].ends_with("StrictHostKeyChecking=yes"));
    assert_eq!(git.learned_host_line(), None);
    assert!(!format!("{git:?}").contains(pinned));
}

#[test]
fn stderr_is_redacted() {
    // A stand-in git that fails, echoing everything it knows.
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("git");
    std::fs::write(
        &bin,
        "#!/bin/sh\necho \"fatal: token $TVGS_GIT_TOKEN askpass $GIT_ASKPASS home $HOME\" >&2\nexit 128\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let git = Git::new(dir.path().join("r.git"), Auth::https(TOKEN))
        .unwrap()
        .with_bin(&bin);
    match git.fetch_instance_branches() {
        Err(Error::Git {
            command,
            status,
            stderr,
        }) => {
            assert_eq!(command, "fetch");
            assert_eq!(status, "exit 128");
            assert!(!stderr.contains(TOKEN), "{stderr}");
            assert!(!stderr.contains("askpass /"), "{stderr}");
            assert!(stderr.contains(REDACTED), "{stderr}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        git.redact(&format!("x {TOKEN} y")),
        format!("x {REDACTED} y")
    );
}

#[test]
fn a_hung_command_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("git");
    // The sleep is a grandchild, which the process group kill takes too.
    std::fs::write(&bin, "#!/bin/sh\nsleep 60 &\nwait\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let git = Git::new(dir.path().join("r.git"), Auth::None)
        .unwrap()
        .with_bin(&bin)
        .with_timeout(Duration::from_millis(300));
    let started = Instant::now();
    let err = git.fetch_instance_branches().unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn no_auth_sets_no_credential_variables() {
    let git = Git::new("/nonexistent", Auth::None).unwrap();
    let env = envs(&git.command(&[OsStr::new("status")]));
    for name in ["GIT_ASKPASS", "GIT_SSH_COMMAND", TOKEN_VAR] {
        assert!(!env.contains_key(name), "{name}");
    }
}
