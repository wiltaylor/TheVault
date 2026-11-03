//! Three servers sharing one bare repo: an edit made on one importer reaches the origin and,
//! through the origin's branch, the other importer.

use serde_json::json;
use testkit::BareRepo;

use crate::instance::{Instance, ids, memo};

#[test]
fn an_edit_on_one_importer_reaches_the_origin_and_the_other_importer() {
    let repo = BareRepo::new();
    let a = Instance::start("A");
    let b = Instance::start("B");
    let c = Instance::start("C");
    let key = a.generate_key();
    b.set_key(&key);
    c.set_key(&key);
    a.configure(&repo.url(), "from memo", "", json!({}));
    b.configure(&repo.url(), "", "from memo", json!({}));
    c.configure(&repo.url(), "", "from memo", json!({}));

    let id = a.create(&memo("Shared three ways", "1", "1"));
    a.sync();
    b.sync();
    c.sync();
    assert_eq!(b.field(&id, "title"), "Shared three ways");
    assert_eq!(c.field(&id, "title"), "Shared three ways");

    c.set(&id, "b", "from C");
    c.sync();
    let report = a.sync();
    assert_eq!(
        ids(&report, "updated"),
        std::slice::from_ref(&id),
        "{report}"
    );
    assert_eq!(a.field(&id, "b"), "from C");
    b.sync();
    assert_eq!(b.field(&id, "b"), "from C");
    assert_eq!(b.authors(&id).len(), 2, "{:?}", b.authors(&id));

    // Every branch settles: C now holds what A publishes, so it stops publishing the note.
    let report = c.sync();
    assert!(ids(&report, "merged").is_empty(), "{report}");
    assert_eq!(report["published"], 0, "{report}");
    for i in [&a, &b, &c] {
        let report = i.sync();
        assert!(!report["pushed"].as_bool().unwrap(), "{}: {report}", i.name);
        assert!(report["clashes"].as_array().unwrap().is_empty());
    }
    assert_eq!(repo.branches().len(), 3);
}
