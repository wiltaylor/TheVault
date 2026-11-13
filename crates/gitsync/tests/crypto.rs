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
