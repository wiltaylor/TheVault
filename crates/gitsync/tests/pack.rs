use gitsync::crypto::OVERHEAD;
use gitsync::pack::{join, open_note, parse_note, seal_note, sha256_hex, split};
use gitsync::{Error, SyncKey};

#[test]
fn a_note_blob_round_trips() {
    let key = SyncKey::generate();
    let layout = gitsync::Layout::new(&key, "vault");
    let wcl = b"title = \"Hello\"\nbody = \"line\\nline\"\n";
    let blob = seal_note(&key, &layout.note("N"), "task", wcl);
    let (ty, bytes) = open_note(&key, &layout.note("N"), &blob).unwrap();
    assert_eq!(ty, "task");
    assert_eq!(bytes, wcl);
    assert!(parse_note(b"no newline").is_err());
}

#[test]
fn an_asset_splits_into_parts_and_joins_back() {
    let key = SyncKey::generate();
    let layout = gitsync::Layout::new(&key, "vault");
    let part = OVERHEAD + 100;
    let bytes: Vec<u8> = (0..1050u32).map(|i| (i * 31 % 251) as u8).collect();
    let parts = split(&key, &layout, "A", &bytes, part);
    assert_eq!(parts.len(), 11);
    assert!(parts.iter().all(|p| p.len() <= part));
    assert_eq!(parts[10].len(), OVERHEAD + 50);
    let sha = sha256_hex(&bytes);
    let back = join(&key, &layout, &parts, "A", bytes.len() as u64, &sha).unwrap();
    assert_eq!(back, bytes);
    // Splitting again gives the same parts, so git stores them once.
    assert_eq!(split(&key, &layout, "A", &bytes, part), parts);
}

#[test]
fn an_exact_multiple_and_an_empty_asset() {
    let key = SyncKey::generate();
    let layout = gitsync::Layout::new(&key, "vault");
    let part = OVERHEAD + 10;
    let bytes = vec![1u8; 30];
    let parts = split(&key, &layout, "A", &bytes, part);
    assert_eq!(parts.len(), 3);
    assert_eq!(
        join(&key, &layout, &parts, "A", 30, &sha256_hex(&bytes)).unwrap(),
        bytes
    );

    let parts = split(&key, &layout, "A", &[], part);
    assert_eq!(parts.len(), 1);
    assert_eq!(
        join(&key, &layout, &parts, "A", 0, &sha256_hex(&[])).unwrap(),
        b""
    );
}

#[test]
fn a_reassembly_that_does_not_match_is_corrupt() {
    let key = SyncKey::generate();
    let layout = gitsync::Layout::new(&key, "vault");
    let part = OVERHEAD + 10;
    let bytes: Vec<u8> = (0..25u8).collect();
    let sha = sha256_hex(&bytes);
    let mut parts = split(&key, &layout, "A", &bytes, part);

    let err = join(&key, &layout, &parts[..2], "A", 25, &sha).unwrap_err();
    assert!(matches!(err, Error::Corrupt { .. }), "{err:?}");

    parts.swap(0, 2);
    let err = join(&key, &layout, &parts, "A", 25, &sha).unwrap_err();
    assert!(matches!(err, Error::KeyMismatch), "{err:?}");

    let err = join(&SyncKey::generate(), &layout, &parts, "A", 25, &sha).unwrap_err();
    assert!(matches!(err, Error::KeyMismatch), "{err:?}");
}

#[test]
fn the_sha256_of_known_bytes() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
