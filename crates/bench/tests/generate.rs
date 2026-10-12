//! The S6 generator: deterministic with a seed, and every file it writes passes a reindex.

use std::collections::BTreeMap;
use std::path::Path;

use bench::generate::{Plan, Spec, generate};

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for dir in [
        "types",
        "taxonomies",
        "notes/person",
        "notes/project",
        "notes/task",
        "notes/article",
    ] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            let name = format!("{dir}/{}", path.file_name().unwrap().to_string_lossy());
            out.insert(name, std::fs::read(&path).unwrap());
        }
    }
    out
}

fn generated(
    notes: usize,
    seed: u64,
    threads: usize,
) -> (tempfile::TempDir, BTreeMap<String, Vec<u8>>) {
    let dir = tempfile::tempdir().unwrap();
    let plan = Plan::new(Spec { notes, seed });
    generate(dir.path(), &plan, threads).unwrap();
    let f = files(dir.path());
    (dir, f)
}

#[test]
fn the_same_seed_writes_the_same_vault() {
    let (_a, one) = generated(600, 7, 1);
    let (_b, two) = generated(600, 7, 8);
    assert_eq!(one.len(), 600 + 6);
    assert!(one == two, "two runs with seed 7 wrote different files");

    let (_c, other) = generated(600, 8, 4);
    assert!(one != other, "seeds 7 and 8 wrote the same files");
}

#[test]
fn every_generated_note_passes_a_reindex() {
    let dir = tempfile::tempdir().unwrap();
    let plan = Plan::new(Spec {
        notes: 2_000,
        seed: 42,
    });
    let generated = generate(dir.path(), &plan, 4).unwrap();
    assert_eq!(
        generated.counts,
        vec![
            ("person", 80),
            ("project", 120),
            ("task", 1_200),
            ("article", 600)
        ]
    );

    let vault = store::Vault::open(dir.path()).unwrap();
    let mut index = index::Index::open(&vault).unwrap();
    let report = index.rebuild(&vault).unwrap();
    assert!(
        report.failures.is_empty(),
        "{:#?}",
        &report.failures[..report.failures.len().min(5)]
    );
    assert_eq!(report.indexed, 2_000);

    // Two-way and peer references agree on both sides, and content is searchable.
    let hits = index.search(plan.word(0), Some(10)).unwrap();
    assert!(!hits.is_empty(), "the most common word found nothing");
    let count = |sql: &str| -> i64 { index.connection().query_row(sql, [], |r| r.get(0)).unwrap() };
    assert!(count("SELECT count(*) FROM links WHERE field = 'project'") > 1_000);
    assert_eq!(
        count(
            "SELECT count(*) FROM links a WHERE a.field = 'project' AND NOT EXISTS
             (SELECT 1 FROM links b WHERE b.src = a.dst AND b.field = 'tasks' AND b.dst = a.src)"
        ),
        0
    );
    assert!(count("SELECT count(*) FROM links WHERE field = 'related'") > 100);
    assert_eq!(
        count(
            "SELECT count(*) FROM links a WHERE a.field = 'related' AND NOT EXISTS
             (SELECT 1 FROM links b WHERE b.src = a.dst AND b.field = 'related' AND b.dst = a.src)"
        ),
        0
    );
}
