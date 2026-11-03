use gitsync::crypto::NAME_LEN;
use gitsync::{Layout, SyncKey};

fn fixed() -> SyncKey {
    SyncKey::from_bytes([7u8; 32])
}

fn is_hex(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn names_are_stable() {
    // Pinned: a change here renames every file in every repo, so it needs a new format.
    let key = fixed();
    assert_eq!(
        key.name("note", "01J00000000000000000000000"),
        "a3bef4b7c53043dccdaec7feb2c19087"
    );
    assert_eq!(
        key.name("instance", "vault-a"),
        "db44cce86595db3f36d5ff848779d19e"
    );
}

#[test]
fn names_depend_on_kind_id_and_key() {
    let key = fixed();
    let n = key.name("note", "x");
    assert_eq!(n.len(), NAME_LEN);
    assert!(is_hex(&n));
    assert_eq!(n, key.name("note", "x"));
    assert_ne!(n, key.name("asset", "x"));
    assert_ne!(n, key.name("note", "y"));
    assert_ne!(n, SyncKey::generate().name("note", "x"));
}

#[test]
fn the_layout_paths() {
    let key = fixed();
    let layout = Layout::new(&key, "vault-a");
    let folder = key.name("instance", "vault-a");
    assert_eq!(layout.folder(), folder);
    assert_eq!(layout.branch(), format!("tv-{folder}"));
    assert_eq!(gitsync::layout::branch(&key, "vault-a"), layout.branch());
    assert_eq!(layout.manifest(), format!("{folder}/manifest"));

    let note = key.name("note", "N1");
    assert_eq!(
        layout.note("N1"),
        format!("{folder}/n/{}/{note}", &note[..2])
    );
    let asset = key.name("asset", "A1");
    assert_eq!(layout.asset_part("A1", 3), format!("{folder}/a/{asset}/3"));
    assert_eq!(
        layout.type_def("task"),
        format!("{folder}/d/{}", key.name("type", "task"))
    );
    assert_eq!(
        layout.taxonomy("tags"),
        format!("{folder}/d/{}", key.name("taxonomy", "tags"))
    );
    assert_ne!(layout.type_def("x"), layout.taxonomy("x"));
}

#[test]
fn a_layout_from_its_branch() {
    let key = fixed();
    let layout = Layout::new(&key, "vault-a");
    let back = Layout::for_branch(&key, &layout.branch()).unwrap();
    assert_eq!(back.folder(), layout.folder());
    assert_eq!(back.note("N1"), layout.note("N1"));
    assert!(Layout::for_branch(&key, "main").is_none());
}

#[test]
fn part_bytes_is_40_mb() {
    assert_eq!(gitsync::PART_BYTES, 40 * 1024 * 1024);
}
