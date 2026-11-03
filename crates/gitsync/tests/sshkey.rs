use gitsync::GITHUB_KNOWN_HOSTS;
use gitsync::sshkey::{generate_ed25519, host_fingerprint, public_key_of};

#[test]
fn a_generated_key_pair() {
    let (private, public) = generate_ed25519("thevault gitsync").unwrap();
    assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    assert!(public.starts_with("ssh-ed25519 AAAA"), "{public}");
    assert!(public.ends_with(" thevault gitsync"), "{public}");
    assert_eq!(public_key_of(&private).unwrap(), public);
    let (other, _) = generate_ed25519("x").unwrap();
    assert_ne!(other, private);
}

#[test]
fn a_pasted_key_that_is_not_a_key_is_refused() {
    assert!(public_key_of("not a key").is_err());
}

#[test]
fn the_fingerprint_of_a_known_hosts_line() {
    let ed25519 = GITHUB_KNOWN_HOSTS
        .lines()
        .find(|l| l.contains("ssh-ed25519"))
        .unwrap();
    // GitHub's published fingerprint.
    assert_eq!(
        host_fingerprint(ed25519).unwrap(),
        "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
    );
    assert_eq!(
        host_fingerprint(&format!("fakehost.test {}", testkit::FAKE_SSH_HOST_KEY)).unwrap(),
        testkit::FAKE_SSH_HOST_FINGERPRINT
    );
    assert!(host_fingerprint("github.com").is_err());
}

#[test]
fn every_github_key_parses() {
    let lines: Vec<&str> = GITHUB_KNOWN_HOSTS.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in lines {
        assert!(line.starts_with("github.com "));
        host_fingerprint(line).unwrap();
    }
}
