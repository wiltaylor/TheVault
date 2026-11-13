//! Adversarial branches exist only in a disposable local bare repository.
use std::collections::BTreeMap;

use gitsync::{Auth, DefEntry, Git, Instance as Named, Layout, Manifest, NoteEntry, SyncKey};
use serde_json::json;
use testkit::BareRepo;

use crate::instance::{Instance, MEMO, memo, s};

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
    assert_eq!(report["skipped_branches"].as_array().unwrap().len(), 1, "{report}");
    assert!(s(&report["skipped_branches"][0]["reason"]).contains(&format!("missing {kind} blob")));
    assert!(!report["warnings"].as_array().unwrap().is_empty(), "{report}");
    assert!(report["archived"].as_array().unwrap().is_empty(), "{report}");
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
