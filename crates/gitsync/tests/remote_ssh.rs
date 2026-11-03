//! Rounds over SSH with a deploy key (git-sync D19), through a fake `ssh` the server is given
//! in `VAULT_GITSYNC_SSH`: the key's public form, `accept-new` only before the host is pinned,
//! and a round that works.

use serde_json::json;
use testkit::{BareRepo, FAKE_SSH_HOST_FINGERPRINT, FakeSsh};

use crate::instance::{Instance, memo, seed};

fn option(call: &[String], name: &str) -> Option<String> {
    call.windows(2)
        .find(|w| w[0] == "-o" && w[1].starts_with(&format!("{name}=")))
        .map(|w| w[1][name.len() + 1..].to_owned())
}

#[test]
fn a_round_over_ssh_learns_the_host_key_then_pins_it() {
    let ssh = FakeSsh::new();
    let repo = BareRepo::new();
    let bin = ssh.bin();
    let a = Instance::start_with("A", seed, &[("VAULT_GITSYNC_SSH", bin.to_str().unwrap())]);
    a.generate_key();
    let made = a.call(
        "POST",
        "/api/integrations/gitsync/ssh-key/generate",
        None,
        200,
    );
    let public = made["public_key"].as_str().unwrap().to_owned();
    assert!(public.starts_with("ssh-ed25519 "), "{public}");
    let status = a.status();
    assert_eq!(status["ssh_key_set"], true);
    assert_eq!(status["ssh_public_key"], public.as_str());
    assert!(!status.to_string().contains("PRIVATE KEY"));
    assert_eq!(status["host_fingerprint"], serde_json::Value::Null);

    a.configure(
        &ssh.url_for(repo.path()),
        "from memo",
        "",
        json!({ "auth": "ssh" }),
    );
    a.create(&memo("Over SSH", "1", "1"));
    let report = a.sync();
    assert_eq!(report["pushed"], true, "{report}");
    assert_eq!(repo.branches().len(), 1);

    let first = ssh.calls();
    assert!(!first.is_empty());
    for call in &first {
        assert_eq!(
            option(call, "StrictHostKeyChecking").as_deref(),
            Some("accept-new")
        );
        assert!(option(call, "UserKnownHostsFile").is_some());
        assert!(call.iter().any(|a| a == "-i"));
    }
    assert_eq!(a.status()["host_fingerprint"], FAKE_SSH_HOST_FINGERPRINT);

    // Pinned now: the next round checks the host strictly, and still works.
    a.create(&memo("Again", "2", "2"));
    assert_eq!(a.sync()["pushed"], true);
    let later = &ssh.calls()[first.len()..];
    assert!(!later.is_empty());
    for call in later {
        assert_eq!(
            option(call, "StrictHostKeyChecking").as_deref(),
            Some("yes")
        );
    }
}
