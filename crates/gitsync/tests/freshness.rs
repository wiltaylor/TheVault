use crate::instance::Instance;
use gitsync::{Auth, Git, Instance as Named, Layout, Manifest, SyncKey};
use serde_json::json;
use std::collections::BTreeMap;
use testkit::BareRepo;

#[test]
fn checkpoints_survive_restart_and_track_peers_independently() {
    let repo = BareRepo::new();
    let mut receiver = Instance::start("receiver");
    let key = SyncKey::parse(&receiver.generate_key()).unwrap();
    receiver.configure(&repo.url(), "from *", "from *", json!({}));
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    let peer = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    let other = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
    let publish = |peer: &str, sequence: u64, name: &str| {
        let layout = Layout::new(&key, peer);
        let mut manifest = Manifest::new(Named {
            vault_id: peer.into(),
            name: name.into(),
        });
        manifest.sequence = sequence;
        let files = BTreeMap::from([(layout.manifest(), manifest.seal(&key))]);
        let tree = git.write_tree(&files).unwrap();
        let commit = git
            .commit_tree(&tree, "synthetic freshness fixture")
            .unwrap();
        git.force_push(&commit, &layout.branch()).unwrap();
    };
    let accepted = |report: serde_json::Value| {
        assert!(
            report["skipped_branches"].as_array().unwrap().is_empty(),
            "{report}"
        );
    };
    let refused = |report: serde_json::Value| {
        assert_eq!(
            report["skipped_branches"].as_array().unwrap().len(),
            1,
            "{report}"
        );
        assert!(
            !report["warnings"].as_array().unwrap().is_empty(),
            "{report}"
        );
    };
    publish(peer, 2, "peer");
    accepted(receiver.sync());
    publish(peer, 3, "peer");
    accepted(receiver.sync());
    receiver.server.restart();
    accepted(receiver.sync());
    publish(peer, 2, "peer");
    refused(receiver.sync());
    publish(peer, 1, "lower");
    refused(receiver.sync());
    publish(peer, 3, "conflicting content");
    refused(receiver.sync());
    publish(peer, 4, "peer");
    publish(other, 1, "other");
    accepted(receiver.sync());
    publish(other, 0, "legacy");
    refused(receiver.sync());
    publish(other, 2, "other");
    accepted(receiver.sync());
    receiver.call(
        "PUT",
        &format!("/api/secrets/gitsync_freshness_receive_{peer}"),
        Some(json!({"value": "broken"})),
        200,
    );
    receiver.server.restart();
    refused(receiver.sync());
}
