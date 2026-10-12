//! D22: init creates the fixed layout; open finds it.

use store::{Error, LAYOUT_DIRS, Vault};
use tempfile::TempDir;

#[test]
fn init_creates_the_layout_and_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("my-vault");
    Vault::init(&root).unwrap();
    std::fs::write(root.join("notes/keep.txt"), "x").unwrap();
    let vault = Vault::init(&root).unwrap();
    for d in LAYOUT_DIRS.iter().chain([".vault"].iter()) {
        assert!(root.join(d).is_dir(), "{d} is missing");
    }
    assert!(root.join("notes/keep.txt").is_file());
    assert_eq!(vault.root(), root.canonicalize().unwrap());
    assert_eq!(vault.server_dir(), vault.root().join(".vault"));
}

#[test]
fn open_refuses_a_folder_that_is_not_a_vault() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(Vault::open(dir.path()), Err(Error::NotAVault(_))));
    assert!(Vault::open(&dir.path().join("missing")).is_err());
}

#[test]
fn open_recreates_layout_folders_git_dropped() {
    let dir = TempDir::new().unwrap();
    Vault::init(dir.path()).unwrap();
    std::fs::remove_dir(dir.path().join("scripts")).unwrap();
    Vault::open(dir.path()).unwrap();
    assert!(dir.path().join("scripts").is_dir());
}
