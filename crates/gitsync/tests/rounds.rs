//! Two servers and a bare repo over `file://` (the main seam): each configured through the API,
//! rounds run with `POST .../run`, and what arrived read back through the notes API.

use std::collections::BTreeMap;

use gitsync::{Auth, Git, Instance as Named, Layout, Manifest, SyncKey};
use serde_json::{Value, json};
use testkit::BareRepo;

use crate::instance::{COURSES, Instance, MEMO, MEMO_PLUS, RECIPE, ids, memo, pair, s};

#[test]
fn the_export_and_import_queries_choose_what_arrives_with_its_id_and_author() {
    let repo = BareRepo::new();
    let (a, b) = pair(
        &repo.url(),
        "from memo where a == \"share\"",
        "from memo where b != \"no\"",
    );
    let shared = a.create(&memo("Shared", "share", "yes"));
    let private = a.create(&memo("Private", "keep", "yes"));
    let unwanted = a.create(&memo("Unwanted", "share", "no"));

    let report = a.sync();
    assert_eq!(report["pushed"], true, "{report}");
    assert_eq!(report["published"], 2, "{report}");
    let report = b.sync();
    assert_eq!(
        ids(&report, "created"),
        std::slice::from_ref(&shared),
        "{report}"
    );

    assert_eq!(b.field(&shared, "title"), "Shared");
    assert_eq!(b.note(&shared).unwrap()["type"], "memo");
    assert!(b.note(&private).is_none(), "an unexported note arrived");
    assert!(
        b.note(&unwanted).is_none(),
        "a note the import query refuses arrived"
    );
    assert_eq!(b.authors(&shared), ["task:gitsync A"]);

    // The activity log has the round, by this instance's name.
    let rounds = b.rounds();
    assert_eq!(rounds.len(), 1, "{rounds:?}");
    assert_eq!(rounds[0]["actor_kind"], "gitsync");
    assert_eq!(rounds[0]["actor"], "B");
    assert_eq!(rounds[0]["detail"]["created"], json!([shared]));

    // A round with nothing to do is not recorded.
    let report = b.sync();
    assert_eq!(report["pushed"], false, "{report}");
    assert!(ids(&report, "created").is_empty());
    assert_eq!(b.rounds().len(), 1);
}

#[test]
fn definitions_are_created_where_missing_and_never_overwritten() {
    let repo = BareRepo::new();
    let a = Instance::start_seeded("A", |v| {
        std::fs::create_dir_all(v.join("types")).unwrap();
        std::fs::create_dir_all(v.join("taxonomies")).unwrap();
        std::fs::write(v.join("types/memo.wcl"), MEMO_PLUS).unwrap();
        std::fs::write(v.join("types/recipe.wcl"), RECIPE).unwrap();
        std::fs::write(v.join("taxonomies/courses.wcl"), COURSES).unwrap();
    });
    let b = Instance::start("B");
    let key = a.generate_key();
    b.set_key(&key);
    a.configure(&repo.url(), "from memo, recipe", "", json!({}));
    b.configure(&repo.url(), "", "from *", json!({}));

    let memo_id = a.create(&memo("A memo", "x", "y"));
    let recipe = a.call(
        "POST",
        "/api/notes?type=recipe",
        Some(json!({ "wcl": "title = \"Soup\"\ncourse = \"Main\"\n" })),
        201,
    );
    let recipe = s(&recipe["id"]).to_owned();
    a.sync();
    let report = b.sync();
    assert_eq!(ids(&report, "created").len(), 2, "{report}");

    assert_eq!(b.field(&recipe, "course"), "Main");
    let read = |path: &str| std::fs::read_to_string(b.server.vault_path().join(path)).unwrap();
    assert!(read("types/recipe.wcl").contains("label = \"Recipe\""));
    assert!(read("taxonomies/courses.wcl").contains("Dessert"));
    // B's own memo type is left as it was.
    assert_eq!(read("types/memo.wcl"), MEMO);
    assert_eq!(b.field(&memo_id, "title"), "A memo");
}

#[test]
fn an_asset_over_the_part_size_arrives_whole() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo", "from memo");
    let bytes: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let asset = a.upload("data.bin", bytes.clone());
    let id = a.create(&format!("title = \"With a file\"\npic = \"{asset}\"\n"));
    let report = a.sync();
    assert!(report["bytes"].as_u64().unwrap() > 20_000, "{report}");
    b.sync();
    assert_eq!(b.field(&id, "pic"), asset.as_str());
    assert_eq!(b.asset(&asset).as_deref(), Some(bytes.as_slice()));

    // Several parts, none over the part size.
    let tree: String = repo
        .branches()
        .iter()
        .map(|b| repo.git(&["ls-tree", "-r", "-l", b]))
        .collect();
    let parts = tree.lines().filter(|l| l.contains("/a/")).count();
    assert!(parts >= 5, "{tree}");
}

#[test]
fn a_link_to_a_note_not_sent_is_dropped_there_and_kept_at_the_origin() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo where a == \"share\"", "from memo");
    a.configure(
        &repo.url(),
        "from memo where a == \"share\"",
        "from memo",
        json!({}),
    );
    b.configure(&repo.url(), "from memo", "from memo", json!({}));
    let target = a.create(&memo("Not sent", "keep", "x"));
    let shared = a.create(&format!(
        "title = \"Links out\"\na = \"share\"\nb = \"one\"\nlink = \"{target}\"\n"
    ));
    let sent_target = a.create(&memo("Sent", "share", "x"));
    let linked = a.create(&format!(
        "title = \"Links in\"\na = \"share\"\nlink = \"{sent_target}\"\n"
    ));
    a.sync();
    b.sync();
    assert!(b.note(&target).is_none());
    assert_eq!(b.field(&shared, "link"), Value::Null);
    assert_eq!(b.field(&shared, "b"), "one");
    // A link to a note sent in the same round is kept.
    assert_eq!(b.field(&linked, "link"), sent_target.as_str());

    // B's edit flows back, and A keeps the link B never had.
    b.set(&shared, "b", "two");
    b.sync();
    let report = a.sync();
    assert_eq!(
        ids(&report, "updated"),
        std::slice::from_ref(&shared),
        "{report}"
    );
    assert_eq!(a.field(&shared, "b"), "two");
    assert_eq!(a.field(&shared, "link"), target.as_str());
}

#[test]
fn edits_flow_back_and_a_clash_goes_to_the_origin_with_one_notification() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo", "from memo");
    let id = a.create(&memo("Clash", "1", "1"));
    a.sync();
    b.sync();

    // B changes a and b, A changes a: a clashes, b does not.
    b.set(&id, "a", "from B");
    b.set(&id, "b", "B's b");
    a.set(&id, "a", "from A");
    let report = b.sync();
    assert_eq!(report["pushed"], true, "{report}");
    let report = a.sync();
    assert_eq!(
        ids(&report, "merged"),
        std::slice::from_ref(&id),
        "{report}"
    );
    assert_eq!(a.field(&id, "a"), "from A");
    assert_eq!(a.field(&id, "b"), "B's b");
    // A won, so A is not told.
    assert!(a.notifications().is_empty(), "{:?}", a.notifications());

    let report = b.sync();
    assert_eq!(report["clashes"][0]["id"], id.as_str(), "{report}");
    assert_eq!(report["clashes"][0]["fields"], json!(["a"]), "{report}");
    assert_eq!(b.field(&id, "a"), "from A");
    assert_eq!(b.field(&id, "b"), "B's b");
    let told = b.notifications();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0]["note"], id.as_str());
    assert!(s(&told[0]["title"]).contains("Clash"), "{told:?}");
    assert!(s(&told[0]["body"]).contains('a'), "{told:?}");
    // B's own value is still in the note's history.
    let versions = b.get(&format!("/api/notes/{id}/history"))["versions"].clone();
    let kept = versions.as_array().unwrap().iter().any(|v| {
        let version = s(&v["version"]);
        let old = b.get(&format!("/api/notes/{id}/history/{version}"));
        old.to_string().contains("from B")
    });
    assert!(kept, "{versions}");

    // Both settle: nothing more moves, and no second notification.
    a.sync();
    let report = b.sync();
    assert!(report["clashes"].as_array().unwrap().is_empty(), "{report}");
    let report = a.sync();
    assert!(ids(&report, "merged").is_empty(), "{report}");
    assert_eq!(b.notifications().len(), 1);
}

#[test]
fn a_note_that_leaves_the_published_set_is_archived_until_it_returns() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo where a == \"share\"", "from memo");
    let leaves = a.create(&memo("Leaves", "share", "x"));
    let user_archived = a.create(&memo("Archived by the user", "share", "x"));
    a.sync();
    b.sync();
    b.at_version("POST", &user_archived, "/archive");

    a.set(&leaves, "a", "private");
    a.set(&user_archived, "a", "private");
    a.sync();
    let report = b.sync();
    assert_eq!(
        ids(&report, "archived"),
        std::slice::from_ref(&leaves),
        "{report}"
    );
    assert!(b.archived(&leaves));
    assert!(b.archived(&user_archived));

    a.set(&leaves, "a", "share");
    a.set(&user_archived, "a", "share");
    a.sync();
    let report = b.sync();
    assert_eq!(
        ids(&report, "unarchived"),
        std::slice::from_ref(&leaves),
        "{report}"
    );
    assert!(!b.archived(&leaves));
    assert!(b.archived(&user_archived), "the user's archive was undone");

    // A note deleted at the origin is archived too.
    a.at_version("DELETE", &leaves, "");
    a.sync();
    let report = b.sync();
    assert_eq!(
        ids(&report, "archived"),
        std::slice::from_ref(&leaves),
        "{report}"
    );
}

#[test]
fn a_note_the_import_query_stops_matching_is_archived_until_it_matches_again() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo", "");
    a.configure(&repo.url(), "from memo", "from memo", json!({}));
    b.configure(
        &repo.url(),
        "from memo",
        "from memo where b != \"no\"",
        json!({}),
    );
    let leaves = a.create(&memo("Leaves", "1", "yes"));
    let user_archived = a.create(&memo("Archived by the user", "1", "yes"));
    let mine = b.create(&memo("B's own", "1", "yes"));
    b.sync();
    a.sync();
    b.sync();
    assert!(!b.archived(&leaves));
    b.at_version("POST", &user_archived, "/archive");

    // A changes the field B's import query filters on: both still published, neither selected.
    a.set(&leaves, "b", "no");
    a.set(&user_archived, "b", "no");
    a.set(&mine, "b", "no");
    a.sync();
    let report = b.sync();
    assert_eq!(
        ids(&report, "archived"),
        std::slice::from_ref(&leaves),
        "{report}"
    );
    assert!(b.archived(&leaves));
    assert!(b.archived(&user_archived));
    // A note B created is always selected, so it is never archived this way.
    assert!(!b.archived(&mine));
    assert_eq!(b.field(&mine, "b"), "no");
    // The archive stays on B: A's copy is not archived.
    a.sync();
    assert!(!a.archived(&leaves));

    a.set(&leaves, "b", "yes");
    a.set(&user_archived, "b", "yes");
    a.sync();
    let report = b.sync();
    assert_eq!(
        ids(&report, "unarchived"),
        std::slice::from_ref(&leaves),
        "{report}"
    );
    assert!(!b.archived(&leaves));
    assert!(b.archived(&user_archived), "the user's archive was undone");
}

#[test]
fn a_deleted_import_stays_deleted_until_restored() {
    let repo = BareRepo::new();
    let (a, b) = pair(&repo.url(), "from memo", "from memo");
    let id = a.create(&memo("Deleted on B", "1", "1"));
    a.sync();
    b.sync();
    b.at_version("DELETE", &id, "");

    a.set(&id, "a", "2");
    a.sync();
    let report = b.sync();
    assert!(ids(&report, "created").is_empty(), "{report}");
    assert!(ids(&report, "updated").is_empty(), "{report}");
    assert!(b.note(&id).is_none(), "the deleted note came back");

    b.call("POST", &format!("/api/trash/{id}/restore"), None, 200);
    let report = b.sync();
    assert_eq!(
        ids(&report, "updated"),
        std::slice::from_ref(&id),
        "{report}"
    );
    assert_eq!(b.field(&id, "a"), "2");
}

#[test]
fn the_size_thresholds_warn_and_then_refuse_the_push_with_one_notification() {
    let repo = BareRepo::new();
    let (a, _b) = pair(&repo.url(), "from memo", "from memo");
    a.create(&memo("Some", "1", "1"));
    a.configure(&repo.url(), "from memo", "", json!({ "warn_bytes": 1 }));
    let report = a.sync();
    let warnings = report["warnings"].to_string();
    assert!(warnings.contains("warning size"), "{report}");
    assert!(
        a.rounds()[0]["detail"]["warnings"]
            .to_string()
            .contains("warning size")
    );

    a.create(&memo("More", "1", "1"));
    a.configure(
        &repo.url(),
        "from memo",
        "",
        json!({ "warn_bytes": 1, "max_bytes": 1 }),
    );
    let before = repo.git(&["rev-parse", &repo.branches()[0]]);
    let status = a.round();
    let error = s(&status["last_error"]);
    assert!(error.contains("refused"), "{status}");
    assert_eq!(repo.git(&["rev-parse", &repo.branches()[0]]), before);
    let told: Vec<Value> = a
        .notifications()
        .into_iter()
        .filter(|n| s(&n["title"]).starts_with("Git sync failed"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    // The same error again notifies no more, though the activity log has each failure.
    let again = a.round();
    assert_eq!(again["last_error"], status["last_error"]);
    let told = a
        .notifications()
        .into_iter()
        .filter(|n| s(&n["title"]).starts_with("Git sync failed"))
        .count();
    assert_eq!(told, 1);
    let failures = a
        .rounds()
        .into_iter()
        .filter(|r| s(&r["summary"]).contains("failed"))
        .count();
    assert_eq!(failures, 2);

    // A round that works clears the error.
    a.configure(&repo.url(), "from memo", "", json!({}));
    a.sync();
    assert_eq!(a.status()["last_error"], Value::Null);
}

/// Push a branch for an instance `vault_id` written by a newer build, with `key`.
fn push_newer_format(repo: &BareRepo, key: &SyncKey, vault_id: &str) {
    let mut manifest = Manifest::new(Named {
        vault_id: vault_id.into(),
        name: "Future".into(),
    });
    manifest.format = gitsync::FORMAT + 1;
    let layout = Layout::new(key, vault_id);
    let mut files = BTreeMap::new();
    files.insert(layout.manifest(), manifest.seal(key));
    let dir = tempfile::tempdir().unwrap();
    let git = Git::new(dir.path().join("repo"), Auth::None).unwrap();
    git.ensure_bare_clone(&repo.url()).unwrap();
    let tree = git.write_tree(&files).unwrap();
    let commit = git.commit_tree(&tree, "future").unwrap();
    git.force_push(&commit, &layout.branch()).unwrap();
}

#[test]
fn a_branch_in_a_newer_format_is_skipped_and_the_own_branch_still_pushed() {
    let repo = BareRepo::new();
    let b = Instance::start("B");
    let key = b.generate_key();
    b.configure(&repo.url(), "from memo", "from memo", json!({}));
    b.create(&memo("Mine", "1", "1"));
    push_newer_format(
        &repo,
        &SyncKey::parse(&key).unwrap(),
        "01JFUTUREVAULT0000000000000",
    );

    let report = b.sync();
    assert_eq!(report["pushed"], true, "{report}");
    let skipped = &report["skipped_branches"];
    assert_eq!(skipped.as_array().unwrap().len(), 1, "{report}");
    assert!(
        s(&skipped[0]["reason"]).contains("update this instance"),
        "{report}"
    );
    assert_eq!(repo.branches().len(), 2);
}

#[test]
fn a_key_mismatch_skips_the_branch() {
    let repo = BareRepo::new();
    let a = Instance::start("A");
    let b = Instance::start("B");
    a.generate_key();
    b.generate_key();
    a.configure(&repo.url(), "from memo", "", json!({}));
    b.configure(&repo.url(), "", "from memo", json!({}));
    let id = a.create(&memo("Locked", "1", "1"));
    a.sync();
    let report = b.sync();
    let skipped = &report["skipped_branches"][0];
    assert!(s(&skipped["reason"]).contains("key mismatch"), "{report}");
    assert!(b.note(&id).is_none());

    // The same key pasted into B ends the mismatch.
    let key = a.generate_key();
    b.set_key(&key);
    a.sync();
    let report = b.sync();
    assert!(
        report["skipped_branches"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert_eq!(b.field(&id, "title"), "Locked");
    // A's branch under the old key is gone: one branch each.
    assert_eq!(repo.branches().len(), 2, "{:?}", repo.branches());
}

#[test]
fn an_unchanged_tree_is_not_pushed() {
    let repo = BareRepo::new();
    let (a, _b) = pair(&repo.url(), "from memo", "");
    a.create(&memo("Steady", "1", "1"));
    assert_eq!(a.sync()["pushed"], true);
    let branch = repo.branches()[0].clone();
    let commit = repo.git(&["rev-parse", &branch]);
    assert_eq!(a.sync()["pushed"], false);
    assert_eq!(repo.git(&["rev-parse", &branch]), commit);
}
