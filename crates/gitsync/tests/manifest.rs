use gitsync::{AssetEntry, DefEntry, Error, FORMAT, Instance, Manifest, NoteEntry, SyncKey};

fn sample() -> Manifest {
    let mut m = Manifest::new(Instance {
        vault_id: "vault-a".into(),
        name: "A".into(),
    });
    m.notes.push(NoteEntry {
        id: "01NOTE".into(),
        note_type: "task".into(),
        origin: "vault-a".into(),
        hash: "ab".repeat(32),
        path: "f/n/aa/aabb".into(),
        assets: vec!["01ASSET".into()],
    });
    m.assets.push(AssetEntry {
        id: "01ASSET".into(),
        filename: "photo.jpg".into(),
        size: 12,
        sha256: "cd".repeat(32),
        parts: vec!["f/a/x/0".into(), "f/a/x/1".into()],
    });
    m.types.push(DefEntry {
        name: "task".into(),
        path: "f/d/t".into(),
    });
    m.taxonomies.push(DefEntry {
        name: "tags".into(),
        path: "f/d/g".into(),
    });
    m.bytes = 1234;
    m
}

#[test]
fn a_manifest_round_trips() {
    let key = SyncKey::generate();
    let m = sample();
    assert_eq!(m.format, FORMAT);
    assert_eq!(FORMAT, 2);
    let blob = m.seal(&key);
    assert_eq!(Manifest::open(&key, &gitsync::Layout::new(&key, "vault-a"), &blob).unwrap(), m);
    // Deterministic, like every blob.
    assert_eq!(m.seal(&key), blob);
}

#[test]
fn the_json_uses_the_spec_field_names() {
    let json = serde_json::to_value(sample()).unwrap();
    for field in [
        "format",
        "instance",
        "notes",
        "assets",
        "types",
        "taxonomies",
        "bytes",
    ] {
        assert!(json.get(field).is_some(), "{field}");
    }
    let note = &json["notes"][0];
    for field in ["id", "type", "origin", "hash", "path", "assets"] {
        assert!(note.get(field).is_some(), "notes.{field}");
    }
    let asset = &json["assets"][0];
    for field in ["id", "filename", "size", "sha256", "parts"] {
        assert!(asset.get(field).is_some(), "assets.{field}");
    }
    assert_eq!(json["instance"]["vault_id"], "vault-a");
}

#[test]
fn a_manifest_under_another_key_is_a_key_mismatch() {
    let blob = sample().seal(&SyncKey::generate());
    assert!(matches!(
        Manifest::open(&SyncKey::generate(), &gitsync::Layout::new(&SyncKey::generate(), "vault-a"), &blob),
        Err(Error::KeyMismatch)
    ));
}

#[test]
fn a_newer_format_is_refused_before_its_shape_is_read() {
    let key = SyncKey::generate();
    // A future manifest whose shape this build cannot read.
    let blob = key.seal(&gitsync::Layout::new(&key, "vault-a").manifest(), br#"{"format": 3, "notes": "something else"}"#);
    match Manifest::open(&key, &gitsync::Layout::new(&key, "vault-a"), &blob) {
        Err(Error::NewerFormat { found, supported }) => {
            assert_eq!((found, supported), (3, FORMAT));
        }
        other => panic!("{other:?}"),
    }
    let err = Manifest::open(&key, &gitsync::Layout::new(&key, "vault-a"), &key.seal(&gitsync::Layout::new(&key, "vault-a").manifest(), b"not json")).unwrap_err();
    assert!(matches!(err, Error::Malformed(_)), "{err:?}");
}
