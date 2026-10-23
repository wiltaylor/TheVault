//! D21: every write emits the canonical form, and writing it again changes nothing.

use std::path::{Path, PathBuf};

use store::{Change, Vault, canonical};
use tempfile::TempDir;

const NOTE: &str = "notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl";

const MESSY: &str = r#"# A comment above everything
content body {   paragraph { text = "Hello" }   // trailing comment
  heading {
        text = "Plan"
        level = 2
  }
}
estimate=3     # hours
title    =   "Ship it"
score = 4
ratio = 2.50
big = 1.5e3
small = -2.5e-4
tags = [ "a" ,
  "b", ]
span = {start:"2026-01-01",end:"2026-01-02"}
term "Blocked"   {   }
source = "line one\n  indented\n"
escaped = "tab\there \"quoted\""
"#;

const CANONICAL: &str = r#"estimate = 3
title    = "Ship it"
score    = 4
ratio    = 2.5
big      = 1500.0
small    = -0.00025
tags     = ["a", "b"]
span     = { start: "2026-01-01", end: "2026-01-02" }
source   = <<'END'
line one
  indented
END
escaped  = "tab\there \"quoted\""

content body {
  paragraph {
    text = "Hello"
  }

  heading {
    text  = "Plan"
    level = 2
  }
}

term "Blocked"
"#;

#[test]
fn a_non_canonical_file_is_written_in_canonical_form() {
    assert_eq!(canonical(MESSY, "messy.wcl").unwrap(), CANONICAL);
}

#[test]
fn canonical_form_is_a_fixed_point() {
    assert_eq!(canonical(CANONICAL, "c.wcl").unwrap(), CANONICAL);
}

#[test]
fn a_heredoc_keeps_its_value_and_picks_a_free_tag() {
    let source = "a = \"END\\n  x\\nEND1\\n\"\n";
    let out = canonical(source, "h.wcl").unwrap();
    assert_eq!(out, "a = <<'END2'\nEND\n  x\nEND1\nEND2\n");
    // A body every line of which is indented cannot be a heredoc: the indent would be stripped.
    let indented = "a = \"  x\\n  y\\n\"\n";
    assert_eq!(canonical(indented, "h.wcl").unwrap(), indented);
}

#[test]
fn writing_through_the_vault_is_canonical_and_byte_identical_twice() {
    let dir = TempDir::new().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    vault.commit(vec![Change::write(NOTE, MESSY)]).unwrap();
    let first = std::fs::read(dir.path().join(NOTE)).unwrap();
    assert_eq!(String::from_utf8(first.clone()).unwrap(), CANONICAL);

    vault.commit(vec![Change::write(NOTE, MESSY)]).unwrap();
    assert_eq!(std::fs::read(dir.path().join(NOTE)).unwrap(), first);

    let reread = vault.read_text(NOTE).unwrap();
    vault.commit(vec![Change::write(NOTE, reread)]).unwrap();
    assert_eq!(std::fs::read(dir.path().join(NOTE)).unwrap(), first);
}

fn example_vault() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/vault-format/vault")
}

fn wcl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            wcl_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "wcl") {
            out.push(path);
        }
    }
}

#[test]
fn every_example_round_trips_through_load_and_canonical_write() {
    let root = example_vault();
    let mut files = Vec::new();
    wcl_files(&root, &mut files);
    files.sort();
    assert_eq!(files.len(), 9, "{files:?}");

    let dir = TempDir::new().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    let mut changes = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        changes.push(Change::write(rel, std::fs::read(file).unwrap()));
    }
    vault.commit(changes).unwrap();

    for file in &files {
        let rel = file.strip_prefix(&root).unwrap().to_str().unwrap();
        let original = std::fs::read_to_string(file).unwrap();
        let loaded = vault.read_wcl(rel).unwrap();
        assert_eq!(
            store::to_canonical(&loaded),
            original,
            "{rel} is not canonical"
        );
        assert_eq!(
            vault.read_text(rel).unwrap(),
            original,
            "{rel} changed on write"
        );
    }
}
