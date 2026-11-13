//! The runner against a bare repo over `file://`.

use std::collections::BTreeMap;

use gitsync::{Auth, Git, Layout, SyncKey};
use testkit::BareRepo;

/// A runner on a fresh local clone of `remote`, and the folder that holds it.
pub fn clone_of(remote: &str, auth: Auth) -> (tempfile::TempDir, Git) {
    let dir = tempfile::tempdir().unwrap();
    let git = Git::new(dir.path().join("gitsync").join("repo"), auth).unwrap();
    git.ensure_bare_clone(remote).unwrap();
    (dir, git)
}

/// A small encrypted tree for `vault_id`: a manifest, two notes and a two-part asset.
pub fn sample_files(key: &SyncKey, vault_id: &str) -> BTreeMap<String, Vec<u8>> {
    let layout = Layout::new(key, vault_id);
    let mut files = BTreeMap::new();
    files.insert(layout.manifest(), key.seal("opaque-git-fixture", b"{\"format\":1}"));
    files.insert(layout.note("N1"), key.seal("opaque-git-fixture", b"task\nbody 1"));
    files.insert(layout.note("N2"), key.seal("opaque-git-fixture", b"task\nbody 2"));
    files.insert(layout.asset_part("A1", 0), key.seal("opaque-git-fixture", b"part 0"));
    files.insert(layout.asset_part("A1", 1), key.seal("opaque-git-fixture", b"part 1"));
    files.insert(layout.type_def("task"), key.seal("opaque-git-fixture", b"type task {}"));
    files
}

/// Builds `files` into a parentless commit and force-pushes it to `branch`; returns the tree.
pub fn publish(git: &Git, files: &BTreeMap<String, Vec<u8>>, branch: &str) -> String {
    let tree = git.write_tree(files).unwrap();
    let commit = git.commit_tree(&tree, "gitsync").unwrap();
    git.force_push(&commit, branch).unwrap();
    tree
}

/// Fetches into `git` and checks it reads back exactly `files` on `branch`.
pub fn assert_fetched(git: &Git, files: &BTreeMap<String, Vec<u8>>, branch: &str, tree: &str) {
    git.fetch_instance_branches().unwrap();
    let fetched = git.fetched_branches().unwrap();
    assert!(
        fetched.iter().any(|(b, _)| b == branch),
        "{branch} in {fetched:?}"
    );
    let rev = format!("refs/remotes/origin/{branch}");
    assert_eq!(git.tree_of(&rev).unwrap().as_deref(), Some(tree));
    let entries = git.read_tree(&rev).unwrap();
    assert_eq!(entries.len(), files.len());
    let oids: Vec<&str> = entries.iter().map(|e| e.oid.as_str()).collect();
    let blobs = git.cat_blobs(&oids).unwrap();
    for (entry, blob) in entries.iter().zip(&blobs) {
        assert_eq!(entry.kind, "blob");
        assert_eq!(entry.mode, "100644");
        assert_eq!(entry.size, Some(blob.len() as u64));
        assert_eq!(&files[&entry.path], blob, "{}", entry.path);
        assert_eq!(&git.cat_blob(&entry.oid).unwrap(), blob);
    }
}

#[test]
fn an_empty_remote_fetches_nothing() {
    let remote = BareRepo::new();
    let (_dir, git) = clone_of(&remote.url(), Auth::None);
    git.fetch_instance_branches().unwrap();
    assert!(git.fetched_branches().unwrap().is_empty());
    assert!(git.list_remote_branches().unwrap().is_empty());
    assert_eq!(git.tree_of("refs/remotes/origin/tv-nothing").unwrap(), None);
}

#[test]
fn build_commit_push_and_fetch_back() {
    let remote = BareRepo::new();
    let key = SyncKey::generate();
    let branch = gitsync::layout::branch(&key, "vault-a");
    let files = sample_files(&key, "vault-a");

    let (_a, a) = clone_of(&remote.url(), Auth::None);
    let tree = publish(&a, &files, &branch);
    assert_eq!(remote.branches(), std::slice::from_ref(&branch));
    // One parentless commit.
    assert_eq!(remote.git(&["rev-list", "--count", &branch]).trim(), "1");

    let (_b, b) = clone_of(&remote.url(), Auth::None);
    assert_fetched(&b, &files, &branch, &tree);
    let listed = b.list_remote_branches().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, branch);
}

#[test]
fn the_same_files_give_the_same_tree_and_a_force_push_replaces_history() {
    let remote = BareRepo::new();
    let key = SyncKey::generate();
    let branch = gitsync::layout::branch(&key, "vault-a");
    let (_a, a) = clone_of(&remote.url(), Auth::None);

    let mut files = sample_files(&key, "vault-a");
    let first = publish(&a, &files, &branch);
    assert_eq!(a.write_tree(&files).unwrap(), first);

    files.insert(
        Layout::new(&key, "vault-a").note("N3"),
        key.seal("opaque-git-fixture", b"task\n3"),
    );
    let second = publish(&a, &files, &branch);
    assert_ne!(first, second);
    assert_eq!(remote.git(&["rev-list", "--count", &branch]).trim(), "1");
    a.fetch_instance_branches().unwrap();
    assert_eq!(
        a.tree_of(&format!("refs/remotes/origin/{branch}"))
            .unwrap()
            .as_deref(),
        Some(second.as_str())
    );
}

#[test]
fn an_empty_tree() {
    let remote = BareRepo::new();
    let (_a, a) = clone_of(&remote.url(), Auth::None);
    assert_eq!(
        a.write_tree(&BTreeMap::new()).unwrap(),
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    );
}

#[test]
fn ensure_bare_clone_is_idempotent_and_follows_a_new_url() {
    let first = BareRepo::new();
    let second = BareRepo::new();
    let (_a, a) = clone_of(&first.url(), Auth::None);
    a.ensure_bare_clone(&first.url()).unwrap();
    a.ensure_bare_clone(&second.url()).unwrap();
    let key = SyncKey::generate();
    publish(&a, &sample_files(&key, "v"), "tv-x");
    assert!(first.branches().is_empty());
    assert_eq!(second.branches(), ["tv-x"]);
}

#[test]
fn branches_other_than_tv_are_not_fetched() {
    let remote = BareRepo::new();
    let key = SyncKey::generate();
    let (_a, a) = clone_of(&remote.url(), Auth::None);
    publish(&a, &sample_files(&key, "v"), "main");
    publish(&a, &sample_files(&key, "v"), "tv-abc");
    let (_b, b) = clone_of(&remote.url(), Auth::None);
    b.fetch_instance_branches().unwrap();
    let names: Vec<String> = b
        .fetched_branches()
        .unwrap()
        .into_iter()
        .map(|p| p.0)
        .collect();
    assert_eq!(names, ["tv-abc"]);
    assert_eq!(b.list_remote_branches().unwrap().len(), 2);
}

#[test]
fn a_branch_gone_from_the_remote_is_pruned() {
    let remote = BareRepo::new();
    let key = SyncKey::generate();
    let (_a, a) = clone_of(&remote.url(), Auth::None);
    publish(&a, &sample_files(&key, "v"), "tv-abc");
    a.fetch_instance_branches().unwrap();
    assert_eq!(a.fetched_branches().unwrap().len(), 1);
    remote.git(&["update-ref", "-d", "refs/heads/tv-abc"]);
    a.fetch_instance_branches().unwrap();
    assert!(a.fetched_branches().unwrap().is_empty());
}
