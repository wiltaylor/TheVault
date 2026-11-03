//! What an observer of the repo sees (user story 8): only `tv-<hex>` branches, each one commit,
//! hex-named files no larger than a part, and no note id or title anywhere.

use serde_json::json;
use testkit::BareRepo;

use crate::instance::{PART_BYTES, memo, pair};

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn the_repo_holds_only_opaque_hex_names_one_commit_per_branch() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo", "from memo");
    b.configure(&repo.url(), "from memo", "from memo", json!({}));
    let bytes: Vec<u8> = (0..10_000u32).map(|i| (i % 253) as u8).collect();
    let asset = a.upload("photo.bin", bytes);
    let mut secrets = vec![asset.clone()];
    for n in 0..3 {
        let title = format!("Secret title {n}");
        let id = a.create(&format!(
            "title = \"{title}\"\na = \"x\"\npic = \"{asset}\"\n"
        ));
        secrets.extend([id, title]);
    }
    let own = b.create(&memo("B's own secret", "y", "y"));
    secrets.extend([own, "B's own secret".to_owned()]);
    for _ in 0..2 {
        a.sync();
        b.sync();
        // A change, so the branches are pushed again.
        a.create(&memo("Another", "z", "z"));
        b.create(&memo("Another on B", "z", "z"));
    }

    let branches = repo.branches();
    assert_eq!(branches.len(), 2, "{branches:?}");
    for branch in &branches {
        let hex = branch.strip_prefix("tv-").expect("a tv- branch");
        assert!(is_hex(hex), "{branch}");
        let commits = repo.git(&["rev-list", "--count", branch]);
        assert_eq!(commits.trim(), "1", "{branch}");
        let tree = repo.git(&["ls-tree", "-r", "-l", branch]);
        for line in tree.lines() {
            let (meta, path) = line.split_once('\t').unwrap();
            let fields: Vec<&str> = meta.split_whitespace().collect();
            let (oid, size) = (fields[2], fields[3].parse::<usize>().unwrap());
            assert!(size <= PART_BYTES, "{line}");
            // The folder kinds (notes, assets, definitions) and the manifest are fixed names;
            // everything else is hex.
            for segment in path.split('/') {
                assert!(
                    is_hex(segment) || ["manifest", "n", "a", "d"].contains(&segment),
                    "{segment} in {path}"
                );
            }
            assert!(path.starts_with(&format!("{hex}/")), "{path}");
            let blob = repo.git(&["cat-file", "-p", oid]);
            for secret in &secrets {
                assert!(!path.contains(secret.as_str()), "{secret} in {path}");
                assert!(!blob.contains(secret.as_str()), "{secret} in {path}");
            }
        }
    }
}
