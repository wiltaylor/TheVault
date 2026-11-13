use crate::instance::Instance;
use crate::instance::{MEMO, MEMO_PLUS};
use gitsync::{Auth, Git, Instance as Named, Layout, Manifest, SyncKey};
use serde_json::json;
use std::collections::BTreeMap;
use testkit::BareRepo;

const ORIGINAL: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const INCOMING: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

fn fixture(key: &SyncKey, peer: &str, sequence: u64, name: &str) -> BTreeMap<String, Vec<u8>> {
    let layout = Layout::new(key, peer);
    let mut manifest = Manifest::new(Named {
        vault_id: peer.into(),
        name: name.into(),
    });
    manifest.sequence = sequence;
    let source = if sequence < 3 { MEMO } else { MEMO_PLUS };
    manifest.types.push(gitsync::DefEntry {
        name: "memo".into(),
        path: layout.type_def("memo"),
        sha256: gitsync::pack::sha256_hex(source.as_bytes()),
    });
    let id = if name == "peer" { ORIGINAL } else { INCOMING };
    let text = format!("title = \"{name}\"\n");
    manifest.notes.push(gitsync::NoteEntry {
        id: id.into(),
        note_type: "memo".into(),
        origin: peer.into(),
        hash: gitsync::pack::sha256_hex(text.as_bytes()),
        path: layout.note(id),
        assets: vec![],
    });
    let blob = if sequence == 0 {
        let mut json = serde_json::to_value(&manifest).unwrap();
        json.as_object_mut().unwrap().remove("sequence");
        key.seal(&layout.manifest(), &serde_json::to_vec(&json).unwrap())
    } else {
        manifest.seal(key)
    };
    BTreeMap::from([
        (layout.manifest(), blob),
        (
            layout.type_def("memo"),
            key.seal(&layout.type_def("memo"), source.as_bytes()),
        ),
        (
            layout.note(id),
            gitsync::pack::seal_note(key, &layout.note(id), "memo", text.as_bytes()),
        ),
    ])
}

fn push(git: &Git, key: &SyncKey, peer: &str, files: &BTreeMap<String, Vec<u8>>) {
    let tree = git.write_tree(files).unwrap();
    let commit = git
        .commit_tree(&tree, "synthetic freshness fixture")
        .unwrap();
    git.force_push(&commit, &Layout::new(key, peer).branch())
        .unwrap();
}

fn refused(report: &serde_json::Value, reason: &str) {
    let skipped = report["skipped_branches"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{report}");
    assert!(
        skipped[0]["reason"].as_str().unwrap().contains(reason),
        "{report}"
    );
    if reason.contains("sequence") || reason.contains("checkpoint") {
        assert!(
            report["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| { w.as_str().is_some_and(|w| w.contains(reason)) }),
            "{report}"
        );
    }
    for field in ["created", "updated", "merged", "archived", "unarchived"] {
        assert!(
            report[field].as_array().unwrap().is_empty(),
            "{field}: {report}"
        );
    }
}

#[test]
fn checkpoints_survive_restart_and_track_peers_independently() {
    let repo = BareRepo::new();
    let mut receiver = Instance::start("receiver");
    let encoded_key = receiver.generate_key();
    let key = SyncKey::parse(&encoded_key).unwrap();
    receiver.configure(&repo.url(), "from *", "from *", json!({}));
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    let peer = "freshness_peer_a";
    let other = "freshness_peer_b";
    let publish = |peer: &str, sequence: u64, name: &str| {
        push(&git, &key, peer, &fixture(&key, peer, sequence, name));
    };
    let accepted = |report: serde_json::Value| {
        assert!(
            report["skipped_branches"].as_array().unwrap().is_empty(),
            "{report}"
        );
    };
    publish(peer, 2, "peer");
    accepted(receiver.sync());
    let original = receiver.note(ORIGINAL).unwrap();
    publish(peer, 3, "peer");
    accepted(receiver.sync());
    receiver.server.restart();
    accepted(receiver.sync());
    publish(peer, 2, "peer");
    refused(&receiver.sync(), "replay or conflicting sequence");
    publish(peer, 1, "lower");
    refused(&receiver.sync(), "replay or conflicting sequence");
    publish(peer, 3, "conflicting content");
    refused(&receiver.sync(), "replay or conflicting sequence");
    assert_eq!(receiver.note(ORIGINAL).unwrap(), original);
    assert!(receiver.note(INCOMING).is_none());
    // Disconnect removes merge bases and clone, but must retain high-water checkpoints.
    receiver.call("DELETE", "/api/integrations/gitsync", None, 204);
    receiver.set_key(&encoded_key);
    receiver.configure(&repo.url(), "from *", "from *", json!({}));
    publish(peer, 2, "peer");
    refused(&receiver.sync(), "replay or conflicting sequence");
    assert_eq!(receiver.note(ORIGINAL).unwrap(), original);
    publish(peer, 4, "peer");
    publish(other, 1, "other");
    accepted(receiver.sync());
    publish(other, 0, "legacy");
    refused(&receiver.sync(), "no freshness sequence");
    publish(other, 2, "other");
    accepted(receiver.sync());
    receiver.call(
        "PUT",
        &format!("/api/secrets/gitsync_freshness_receive_{peer}"),
        Some(json!({"value": "broken"})),
        200,
    );
    receiver.server.restart();
    refused(&receiver.sync(), "corrupt git-sync freshness checkpoint");
}

#[test]
fn older_valid_definition_is_refused_and_unreferenced_junk_does_not_pin_checkpoint() {
    let repo = BareRepo::new();
    let receiver = Instance::start("receiver");
    let key = SyncKey::parse(&receiver.generate_key()).unwrap();
    receiver.configure(&repo.url(), "", "from *", json!({}));
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    let peer = "tamper_peer";
    let layout = Layout::new(&key, peer);
    push(&git, &key, peer, &fixture(&key, peer, 2, "peer"));
    receiver.sync();
    let original = receiver.note(ORIGINAL).unwrap();
    let genuine = fixture(&key, peer, 3, "replacement");
    let mut tampered = genuine.clone();
    // Genuine unseen manifest N, older valid ciphertext from N-1 at the same path.
    tampered.insert(
        layout.type_def("memo"),
        fixture(&key, peer, 2, "peer")[&layout.type_def("memo")].clone(),
    );
    push(&git, &key, peer, &tampered);
    refused(&receiver.sync(), "plaintext hash mismatch");
    assert_eq!(receiver.note(ORIGINAL).unwrap(), original);
    assert!(receiver.note(INCOMING).is_none());
    let mut junk = genuine.clone();
    junk.insert("unreferenced-junk".into(), b"not authenticated".to_vec());
    push(&git, &key, peer, &junk);
    let report = receiver.sync();
    assert!(
        report["skipped_branches"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(receiver.note(INCOMING).is_some());
    push(&git, &key, peer, &genuine);
    let report = receiver.sync();
    assert!(
        report["skipped_branches"].as_array().unwrap().is_empty(),
        "{report}"
    );
    // Missing definition hashes from an older peer are refused, not accepted once.
    let mut legacy = fixture(&key, peer, 4, "legacy");
    let mut manifest: serde_json::Value = serde_json::from_slice(
        &key.open(&layout.manifest(), &legacy[&layout.manifest()])
            .unwrap(),
    )
    .unwrap();
    manifest["types"][0]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    legacy.insert(
        layout.manifest(),
        key.seal(&layout.manifest(), &serde_json::to_vec(&manifest).unwrap()),
    );
    push(&git, &key, peer, &legacy);
    refused(&receiver.sync(), "upgrade every peer and republish");
}

#[test]
fn publisher_seeds_from_authenticated_own_remote_counter_after_local_rollback() {
    let repo = BareRepo::new();
    let publisher = Instance::start("publisher");
    let key = SyncKey::parse(&publisher.generate_key()).unwrap();
    publisher.configure(&repo.url(), "from *", "", json!({}));
    publisher.create("title = \"local retained data\"\n");
    publisher.sync();
    let info = publisher.get("/api/sync/info");
    let peer = info["vault_id"].as_str().unwrap();
    let layout = Layout::new(&key, peer);
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    // A remote ahead of local state models a restored database or a stopped copy.
    // Also omit definition hashes to prove the upgrade can republish its own old branch.
    let mut files = fixture(&key, peer, 50, "old publisher");
    let mut manifest: serde_json::Value = serde_json::from_slice(
        &key.open(&layout.manifest(), &files[&layout.manifest()])
            .unwrap(),
    )
    .unwrap();
    manifest["types"][0]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    files.insert(
        layout.manifest(),
        key.seal(&layout.manifest(), &serde_json::to_vec(&manifest).unwrap()),
    );
    push(&git, &key, peer, &files);
    publisher.sync();
    git.fetch_instance_branches().unwrap();
    let read = || {
        let entries = git
            .read_tree(&format!("refs/remotes/origin/{}", layout.branch()))
            .unwrap();
        let entry = entries
            .iter()
            .find(|e| e.path == layout.manifest())
            .unwrap();
        Manifest::open(&key, &layout, &git.cat_blob(&entry.oid).unwrap()).unwrap()
    };
    assert_eq!(read().sequence, 51);
    let report = publisher.sync();
    assert_eq!(report["pushed"], false, "{report}");
    git.fetch_instance_branches().unwrap();
    assert_eq!(read().sequence, 51);
}
