//! The runner through the fake `ssh` against a bare repo.

use gitsync::sshkey::{generate_ed25519, host_fingerprint};
use gitsync::{Auth, Error, Git, SyncKey};
use testkit::{BareRepo, FAKE_SSH_HOST, FAKE_SSH_HOST_FINGERPRINT, FakeSsh};

use crate::git_file::{assert_fetched, publish, sample_files};

fn runner(ssh: &FakeSsh, remote: &str, known_hosts: &str) -> (tempfile::TempDir, Git) {
    let (private, _) = generate_ed25519("test").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let git = Git::new(
        dir.path().join("repo"),
        Auth::Ssh {
            private_key: private,
            known_hosts: known_hosts.into(),
        },
    )
    .unwrap()
    .with_ssh_bin(ssh.bin());
    git.ensure_bare_clone(remote).unwrap();
    (dir, git)
}

fn option(call: &[String], name: &str) -> Option<String> {
    call.windows(2)
        .find(|w| w[0] == "-o" && w[1].starts_with(&format!("{name}=")))
        .map(|w| w[1][name.len() + 1..].to_owned())
}

#[test]
fn a_round_trip_over_ssh_learns_then_pins_the_host_key() {
    let ssh = FakeSsh::new();
    let remote = BareRepo::new();
    let url = ssh.url_for(remote.path());
    let key = SyncKey::generate();
    let branch = gitsync::layout::branch(&key, "vault-c");
    let files = sample_files(&key, "vault-c");

    // Nothing pinned: accept-new, and the learned line comes back.
    let (_a, a) = runner(&ssh, &url, "");
    a.fetch_instance_branches().unwrap();
    let tree = publish(&a, &files, &branch);
    assert_eq!(remote.branches(), std::slice::from_ref(&branch));
    let learned = a.learned_host_line().expect("a learned host line");
    assert!(
        learned.starts_with(&format!("{FAKE_SSH_HOST} ")),
        "{learned}"
    );
    assert_eq!(
        host_fingerprint(&learned).unwrap(),
        FAKE_SSH_HOST_FINGERPRINT
    );

    let calls = ssh.calls();
    assert!(!calls.is_empty());
    for call in &calls {
        assert_eq!(
            option(call, "StrictHostKeyChecking").as_deref(),
            Some("accept-new")
        );
        assert_eq!(option(call, "IdentitiesOnly").as_deref(), Some("yes"));
        assert!(call.iter().any(|a| a == "-i"));
        assert!(
            call.last()
                .unwrap()
                .contains(&remote.path().display().to_string())
        );
    }

    // Pinned: strict checking, and the round still works.
    let before = calls.len();
    let (_b, b) = runner(&ssh, &url, &learned);
    assert_fetched(&b, &files, &branch, &tree);
    assert_eq!(b.learned_host_line(), None);
    let calls = ssh.calls();
    assert!(calls.len() > before);
    for call in &calls[before..] {
        assert_eq!(
            option(call, "StrictHostKeyChecking").as_deref(),
            Some("yes")
        );
    }
}

#[test]
fn a_pinned_key_that_does_not_match_is_refused() {
    let ssh = FakeSsh::new();
    let remote = BareRepo::new();
    let url = ssh.url_for(remote.path());
    let (_a, a) = runner(
        &ssh,
        &url,
        &format!(
            "{FAKE_SSH_HOST} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
        ),
    );
    match a.fetch_instance_branches() {
        Err(Error::Git { stderr, .. }) => {
            assert!(stderr.contains("Host key verification failed"), "{stderr}");
            // The key path is redacted.
            assert!(!stderr.contains("/tv-gitsync-"), "{stderr}");
        }
        other => panic!("{other:?}"),
    }
}
