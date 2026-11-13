use gitsync::crypto::{BLOB_VERSION, KEY_PREFIX, MAGIC, OVERHEAD};
use gitsync::{Error, SyncKey};

fn fixed() -> SyncKey {
    SyncKey::from_bytes([7u8; 32])
}

#[test]
fn a_blob_round_trips() {
    let key = SyncKey::generate();
    let blob = key.seal("test-context", b"hello notes");
    assert_eq!(&blob[..4], MAGIC);
    assert_eq!(blob[4], BLOB_VERSION);
    assert_eq!(blob.len(), OVERHEAD + b"hello notes".len());
    assert_eq!(key.open("test-context", &blob).unwrap(), b"hello notes");
    assert_eq!(key.open("test-context", &key.seal("test-context", b"")).unwrap(), b"");
}

#[test]
fn the_same_plaintext_gives_the_same_blob() {
    let key = fixed();
    assert_eq!(key.seal("test-context", b"same"), key.seal("test-context", b"same"));
    assert_ne!(key.seal("test-context", b"same"), key.seal("test-context", b"other"));
    // Another key seals the same plaintext differently.
    assert_ne!(
        key.seal("test-context", b"same"),
        SyncKey::from_bytes([8u8; 32]).seal("test-context", b"same")
    );
}

#[test]
fn the_plaintext_does_not_show_in_the_blob() {
    let key = fixed();
    let blob = key.seal("test-context", b"a very secret title");
    assert!(!blob.windows(6).any(|w| w == b"secret"));
}

#[test]
fn a_wrong_key_is_a_key_mismatch() {
    let blob = fixed().seal("test-context", b"hello");
    let err = SyncKey::generate().open("test-context", &blob).unwrap_err();
    assert!(matches!(err, Error::KeyMismatch), "{err:?}");
}

#[test]
fn an_altered_blob_is_a_key_mismatch() {
    let key = fixed();
    let mut blob = key.seal("test-context", b"hello");
    let last = blob.len() - 1;
    blob[last] ^= 1;
    assert!(matches!(key.open("test-context", &blob), Err(Error::KeyMismatch)));
    // So is a changed header, which is authenticated too.
    let mut blob = key.seal("test-context", b"hello");
    blob[0] = b'X';
    assert!(matches!(key.open("test-context", &blob), Err(Error::Malformed(_))));
}

#[test]
fn a_short_or_foreign_blob_is_malformed() {
    let key = fixed();
    assert!(matches!(key.open("test-context", b"TVGS"), Err(Error::Malformed(_))));
    let mut blob = key.seal("test-context", b"hello");
    blob[4] = 9;
    assert!(matches!(key.open("test-context", &blob), Err(Error::Malformed(_))));
}

#[test]
fn the_key_text_round_trips() {
    let key = SyncKey::generate();
    let text = key.to_text();
    assert!(text.starts_with(KEY_PREFIX));
    assert!(!text.contains('='), "no padding: {text}");
    // 32 bytes is 43 base64 characters without padding.
    assert_eq!(text.len(), KEY_PREFIX.len() + 43);
    assert_eq!(SyncKey::parse(&text).unwrap(), key);
    assert_eq!(SyncKey::parse(&format!("  {text}\n")).unwrap(), key);
}

#[test]
fn the_key_text_of_known_bytes() {
    let text = fixed().to_text();
    assert_eq!(text, "tvsync1-BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc");
}

#[test]
fn malformed_key_text_is_refused() {
    for bad in [
        "",
        "tvsync2-BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
        "tvsync1-not base64!",
        "tvsync1-BwcHBw",
        "tvsync1-BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=",
    ] {
        let err = SyncKey::parse(bad).unwrap_err();
        assert!(matches!(err, Error::BadKey(_)), "{bad:?}: {err:?}");
    }
}

#[test]
fn debug_hides_the_key() {
    assert_eq!(format!("{:?}", fixed()), "SyncKey(..)");
}

#[test]
fn canonical_context_binds_instance_kind_id_and_part_position() {
    let key = fixed();
    let a = gitsync::Layout::new(&key, "a");
    let b = gitsync::Layout::new(&key, "b");
    let contexts = [
        a.manifest(), b.manifest(), a.note("one"), a.note("two"), b.note("one"),
        a.type_def("one"), a.type_def("two"), a.taxonomy("one"),
        a.asset_part("one", 0), a.asset_part("one", 1), a.asset_part("two", 0),
    ];
    for context in &contexts {
        let blob = key.seal(context, b"identical content");
        assert_eq!(blob, key.seal(context, b"identical content"));
        assert_eq!(key.open(context, &blob).unwrap(), b"identical content");
        for other in &contexts {
            if other != context {
                assert!(matches!(key.open(other, &blob), Err(Error::KeyMismatch)));
                // Context enters nonce derivation too, avoiding nonce reuse across AADs.
                assert_ne!(&blob[5..29], &key.seal(other, b"identical content")[5..29]);
            }
        }
    }
}

#[test]
fn legacy_blob_is_refused_without_fallback() {
    let key = fixed();
    let mut blob = key.seal("context", b"legacy");
    blob[4] = 1;
    let err = key.open("context", &blob).unwrap_err();
    assert!(err.to_string().contains("upgrade every peer"));
}
