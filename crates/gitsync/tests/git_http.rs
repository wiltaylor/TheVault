//! The runner against `git http-backend` behind a token.

use gitsync::{Auth, Error, SyncKey};
use testkit::GitHttpServer;

use crate::git_file::{assert_fetched, clone_of, publish, sample_files};

const TOKEN: &str = "github_pat_right_ABC123";

#[test]
fn the_right_token_pushes_and_fetches() {
    let server = GitHttpServer::start(TOKEN);
    let key = SyncKey::generate();
    let branch = gitsync::layout::branch(&key, "vault-a");
    let files = sample_files(&key, "vault-a");

    let (_a, a) = clone_of(&server.url(), Auth::https(TOKEN));
    a.fetch_instance_branches().unwrap();
    let tree = publish(&a, &files, &branch);
    assert_eq!(server.branches(), std::slice::from_ref(&branch));

    let (_b, b) = clone_of(&server.url(), Auth::https(TOKEN));
    assert_fetched(&b, &files, &branch, &tree);
    assert!(server.served() > 0);
}

#[test]
fn a_wrong_token_is_refused_and_redacted() {
    let server = GitHttpServer::start(TOKEN);
    let wrong = "github_pat_wrong_XYZ789";
    let (_a, a) = clone_of(&server.url(), Auth::https(wrong));
    match a.fetch_instance_branches() {
        Err(Error::Git { stderr, .. }) => {
            assert!(!stderr.contains(wrong), "{stderr}");
            assert!(!stderr.contains(TOKEN), "{stderr}");
            assert!(
                stderr.contains("Authentication failed") || stderr.contains("401"),
                "{stderr}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(server.refused() > 0);
    assert_eq!(server.served(), 0);

    let key = SyncKey::generate();
    let tree = a.write_tree(&sample_files(&key, "v")).unwrap();
    let commit = a.commit_tree(&tree, "gitsync").unwrap();
    let err = a.force_push(&commit, "tv-x").unwrap_err();
    assert!(!err.to_string().contains(wrong), "{err}");
    assert!(server.branches().is_empty());
}

#[test]
fn no_token_is_refused_without_prompting() {
    let server = GitHttpServer::start(TOKEN);
    let (_a, a) = clone_of(&server.url(), Auth::None);
    let err = a.fetch_instance_branches().unwrap_err();
    assert!(matches!(err, Error::Git { .. }), "{err:?}");
}
