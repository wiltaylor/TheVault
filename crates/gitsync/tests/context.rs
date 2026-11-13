//! Adversarial branches exist only in a disposable local bare repository.
use std::collections::BTreeMap;

use gitsync::{AssetEntry, Auth, DefEntry, Git, Instance as Named, Layout, Manifest, NoteEntry, SyncKey};
use serde_json::json;
use testkit::BareRepo;

use crate::instance::{Instance, MEMO, memo, s};

#[test]
fn unchanged_note_retries_failed_asset_then_skips_present_assets() {
    retry_assets(1, true);
}

#[test]
fn unchanged_note_keeps_warning_for_invalid_asset_without_writing_it() {
    retry_assets(1, false);
}

#[test]
fn unchanged_notes_share_one_round_asset_retry_cap() {
    retry_assets(40, false);
}

fn retry_assets(count: usize, repair: bool) {
    let repo = BareRepo::new();
    let receiver = Instance::start("receiver");
    let key = SyncKey::parse(&receiver.generate_key()).unwrap();
    receiver.configure(&repo.url(), "from *", "from *", json!({}));
    let peer = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    let layout = Layout::new(&key, peer);
    let mut manifest = Manifest::new(Named {
        vault_id: peer.into(),
        name: "peer".into(),
    });
    manifest.types.push(DefEntry {
        name: "memo".into(),
        path: layout.type_def("memo"),
    });
    let mut files = BTreeMap::from([(
        layout.type_def("memo"),
        key.seal(&layout.type_def("memo"), MEMO.as_bytes()),
    )]);
    let bytes = b"authenticated asset";
    for i in 0..count {
        let id = format!("01JB8Z4K2MQ7V3X9T5R6W8{:05}", i);
        let asset = format!("01JB8Z4K2MQ7V3X9T5R6W9{:05}", i);
        let text = format!("title = \"Asset fixture\"\npic = \"{asset}\"\n");
        manifest.notes.push(NoteEntry {
            id: id.clone(),
            note_type: "memo".into(),
            origin: peer.into(),
            hash: gitsync::pack::sha256_hex(text.as_bytes()),
            path: layout.note(&id),
            assets: vec![asset.clone()],
        });
        let path = layout.asset_part(&asset, 0);
        manifest.assets.push(AssetEntry {
            id: asset,
            filename: "fixture.bin".into(),
            size: bytes.len() as u64,
            sha256: gitsync::pack::sha256_hex(bytes),
            parts: vec![path.clone()],
        });
        files.insert(
            layout.note(&id),
            gitsync::pack::seal_note(&key, &layout.note(&id), "memo", text.as_bytes()),
        );
        // Valid ciphertext under the wrong context must never reach the asset store.
        files.insert(path, key.seal("wrong context", bytes));
    }
    files.insert(layout.manifest(), manifest.seal(&key));
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    let publish = |files: &BTreeMap<String, Vec<u8>>| {
        let tree = git.write_tree(files).unwrap();
        let commit = git.commit_tree(&tree, "asset retry fixture").unwrap();
        git.force_push(&commit, &layout.branch()).unwrap();
    };
    publish(&files);
    let first = receiver.sync();
    assert_eq!(first["created"].as_array().unwrap().len(), count, "{first}");
    assert_eq!(first["warnings"].as_array().unwrap().len(), count, "{first}");
    for asset in &manifest.assets {
        assert!(receiver.asset(&asset.id).is_none());
        if repair {
            let path = &asset.parts[0];
            files.insert(path.clone(), key.seal(path, bytes));
        }
    }
    if repair {
        publish(&files);
    }
    for _ in 0..2 {
        let later = receiver.sync();
        assert!(later["created"].as_array().unwrap().is_empty(), "{later}");
        assert!(later["updated"].as_array().unwrap().is_empty(), "{later}");
        assert_eq!(
            later["warnings"].as_array().unwrap().len(),
            if repair { 0 } else { count.min(32) },
            "{later}"
        );
        for asset in &manifest.assets {
            assert_eq!(
                receiver.asset(&asset.id).as_deref(),
                repair.then_some(bytes.as_slice())
            );
        }
        for note in &manifest.notes {
            assert!(receiver.note(&note.id).is_some());
        }
    }
    if repair {
        // Corrupt the remote part again. A locally present asset must not be fetched/opened.
        let path = &manifest.assets[0].parts[0];
        files.insert(path.clone(), key.seal("wrong context", bytes));
        publish(&files);
        let present = receiver.sync();
        assert!(
            present["warnings"].as_array().unwrap().is_empty(),
            "{present}"
        );
        assert_eq!(
            receiver.asset(&manifest.assets[0].id).as_deref(),
            Some(bytes.as_slice())
        );
    }
}

#[test]
fn missing_manifest_note_blob_skips_branch_without_archiving() {
    missing_manifest_blob("note");
}

#[test]
fn missing_manifest_definition_blob_skips_branch_without_archiving() {
    missing_manifest_blob("definition");
}

fn missing_manifest_blob(kind: &str) {
    let repo = BareRepo::new();
    let receiver = Instance::start("receiver");
    let key = SyncKey::parse(&receiver.generate_key()).unwrap();
    receiver.configure(&repo.url(), "from *", "from *", json!({}));
    let peer = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    let id = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
    let new_id = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";
    let layout = Layout::new(&key, peer);
    let text = memo("Earlier import", "share", "yes");
    let mut manifest = Manifest::new(Named {
        vault_id: peer.into(),
        name: "peer".into(),
    });
    manifest.types.push(DefEntry {
        name: "memo".into(),
        path: layout.type_def("memo"),
    });
    manifest.notes.push(NoteEntry {
        id: id.into(),
        note_type: "memo".into(),
        origin: peer.into(),
        hash: gitsync::pack::sha256_hex(text.as_bytes()),
        path: layout.note(id),
        assets: vec![],
    });
    let mut files = BTreeMap::from([
        (layout.manifest(), manifest.seal(&key)),
        (
            layout.type_def("memo"),
            key.seal(&layout.type_def("memo"), MEMO.as_bytes()),
        ),
        (
            layout.note(id),
            gitsync::pack::seal_note(&key, &layout.note(id), "memo", text.as_bytes()),
        ),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    let publish = |files: &BTreeMap<String, Vec<u8>>| {
        let tree = git.write_tree(files).unwrap();
        let commit = git.commit_tree(&tree, "missing blob fixture").unwrap();
        git.force_push(&commit, &layout.branch()).unwrap();
    };
    publish(&files);
    let report = receiver.sync();
    assert!(report["skipped_branches"].as_array().unwrap().is_empty());
    assert_eq!(receiver.field(id, "title"), "Earlier import");
    assert!(!receiver.archived(id));

    let mut new_entry = manifest.notes[0].clone();
    new_entry.id = new_id.into();
    new_entry.path = layout.note(new_id);
    manifest.notes.push(new_entry);
    files.insert(layout.manifest(), manifest.seal(&key));
    files.insert(
        layout.note(new_id),
        gitsync::pack::seal_note(&key, &layout.note(new_id), "memo", text.as_bytes()),
    );
    files.remove(&if kind == "note" {
        layout.note(id)
    } else {
        layout.type_def("memo")
    });
    publish(&files);
    let report = receiver.sync();
    assert_eq!(
        report["skipped_branches"].as_array().unwrap().len(),
        1,
        "{report}"
    );
    assert!(s(&report["skipped_branches"][0]["reason"]).contains(&format!("missing {kind} blob")));
    assert!(
        !report["warnings"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(
        report["archived"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(receiver.note(new_id).is_none());
    assert_eq!(receiver.field(id, "title"), "Earlier import");
    assert!(!receiver.archived(id));
}

#[test]
fn altered_context_and_manifest_are_refused_then_upgraded_peer_imports() {
    let repo = BareRepo::new();
    let receiver = Instance::start("receiver");
    let key = SyncKey::parse(&receiver.generate_key()).unwrap();
    receiver.configure(&repo.url(), "from *", "from *", json!({}));
    let peer = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    let id = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
    let layout = Layout::new(&key, peer);
    let text = memo("Bound note", "share", "yes");
    let mut manifest = Manifest::new(Named {
        vault_id: peer.into(),
        name: "peer".into(),
    });
    manifest.types.push(DefEntry {
        name: "memo".into(),
        path: layout.type_def("memo"),
    });
    manifest.notes.push(NoteEntry {
        id: id.into(),
        note_type: "memo".into(),
        origin: peer.into(),
        hash: gitsync::pack::sha256_hex(text.as_bytes()),
        path: layout.note(id),
        assets: vec![],
    });
    let temp = tempfile::tempdir().unwrap();
    let git = Git::new(temp.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    for case in [
        "note swap",
        "definition swap",
        "hash",
        "type",
        "path",
        "legacy blob",
        "legacy manifest",
        "valid",
    ] {
        let mut m = manifest.clone();
        let note_path = if case == "note swap" {
            layout.note("another")
        } else {
            layout.note(id)
        };
        let def_path = if case == "definition swap" {
            layout.taxonomy("memo")
        } else {
            layout.type_def("memo")
        };
        if case == "hash" {
            m.notes[0].hash = "00".repeat(32);
        }
        if case == "type" {
            m.notes[0].note_type = "task".into();
        }
        if case == "path" {
            m.notes[0].id = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC".into();
        }
        if case == "legacy manifest" {
            m.format = 1;
        }
        let mut blob = m.seal(&key);
        if case == "legacy blob" {
            blob[4] = 1;
        }
        let files = BTreeMap::from([
            (layout.manifest(), blob),
            (
                layout.type_def("memo"),
                key.seal(&def_path, MEMO.as_bytes()),
            ),
            (
                layout.note(id),
                gitsync::pack::seal_note(&key, &note_path, "memo", text.as_bytes()),
            ),
        ]);
        let tree = git.write_tree(&files).unwrap();
        let commit = git.commit_tree(&tree, "context fixture").unwrap();
        git.force_push(&commit, &layout.branch()).unwrap();
        let report = receiver.sync();
        if case == "valid" {
            assert!(
                report["skipped_branches"].as_array().unwrap().is_empty(),
                "{report}"
            );
            assert_eq!(receiver.field(id, "title"), "Bound note");
        } else {
            assert_eq!(
                report["skipped_branches"].as_array().unwrap().len(),
                1,
                "{case}: {report}"
            );
            assert!(receiver.note(id).is_none(), "{case}");
            if case.starts_with("legacy") {
                assert!(s(&report["skipped_branches"][0]["reason"]).contains("upgrade"));
            }
        }
    }
    assert_eq!(receiver.sync()["pushed"], false);
}
