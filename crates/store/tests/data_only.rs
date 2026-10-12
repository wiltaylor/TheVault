//! D20: loading or submitting a file with an expression or an import fails, naming the item.

use store::{Change, Error, Vault, parse_data_only};
use tempfile::TempDir;

const NOTE: &str = "notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl";

fn data_only_error(source: &str) -> store::DataOnlyError {
    match parse_data_only(source, "note.wcl") {
        Err(Error::DataOnly(e)) => e,
        other => panic!("expected a data-only error for {source:?}, got {other:?}"),
    }
}

#[test]
fn the_invalid_example_is_rejected_naming_the_field() {
    let source = include_str!("../../../docs/vault-format/invalid/expression.wcl");
    let e = data_only_error(source);
    assert_eq!(e.item, "estimate");
    assert_eq!(e.line, 4);
    assert!(e.message.contains('+'), "{e}");
    assert!(e.to_string().starts_with("note.wcl:4: estimate:"), "{e}");
}

#[test]
fn an_import_is_rejected_naming_it() {
    let e = data_only_error("title = \"x\"\nimport \"./other.wcl\"\n");
    assert_eq!(e.item, "`import \"./other.wcl\"`");
    assert_eq!(e.line, 2);
    assert!(e.message.contains("import"), "{e}");
}

#[test]
fn every_kind_of_expression_is_rejected_naming_the_item() {
    // (source, item the diagnostic must name)
    let cases = [
        ("a = 1 + 2", "a"),
        ("a = len(\"x\")", "a"),
        ("a = !true", "a"),
        ("a = b", "a"),
        ("a = $\"hi ${b}\"", "a"),
        ("a = 3u32", "a"),
        ("a = 5MiB", "a"),
        ("a = 0xFF", "a"),
        ("a = 1_000", "a"),
        ("a = 1.5f32", "a"),
        ("a = utf8\"x\"", "a"),
        ("a = ascii\"x\"", "a"),
        ("a = none", "a"),
        ("a = :gold", "a"),
        ("a = [1, b]", "a"),
        ("a = { x: 1 + 1 }", "a.x"),
        ("a = if true { 1 } else { 2 }", "a"),
        ("a = - 1", "a"),
        (
            "content body {\n  paragraph {\n    text = x\n  }\n}",
            "content[body].paragraph.text",
        ),
        ("@doc(\"x\")\na = 1", "a"),
        ("let base = 3\na = 1", "`let base = 3`"),
        ("fn f() -> i64 { 1 }", "`fn f() -> i64 { 1 }`"),
        ("type T { a: i64 }", "`type T { a: i64 }`"),
        ("symbol_set S { a }", "`symbol_set S { a }`"),
        ("import <wdoc/core.wcl>", "`import <wdoc/core.wcl>`"),
        ("rows:\n  | 1 | 2 |", "`rows:`"),
    ];
    for (source, item) in cases {
        let e = data_only_error(source);
        assert_eq!(e.item, item, "{source:?}: {e}");
    }
}

#[test]
fn plain_data_is_accepted() {
    let source = r#"title = "Plain"
count = -3
ratio = -0.5e-3
done = false
tags = ["a", "b",]
span = { start: "2026-01-01", end: "2026-01-02" }
text = "quote \" backslash \\ tab \t"
body = <<'END'
  raw \d text
END
term Active/In-progress
term "Blocked" {
  colour = "red"
}
"#;
    parse_data_only(source, "ok.wcl").unwrap();
}

#[test]
fn submitting_an_expression_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    let good = "notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZB.wcl";
    let err = vault
        .commit(vec![
            Change::write(good, "title = \"fine\"\n"),
            Change::write(NOTE, "title = \"x\"\nestimate = 1 + 2\n"),
        ])
        .unwrap_err();
    match err {
        Error::DataOnly(e) => {
            assert_eq!(e.file, NOTE);
            assert_eq!(e.item, "estimate");
        }
        other => panic!("expected a data-only error, got {other:?}"),
    }
    assert!(!dir.path().join(good).exists());
    assert!(!dir.path().join(NOTE).exists());
}

#[test]
fn loading_an_expression_or_an_import_from_disk_fails() {
    let dir = TempDir::new().unwrap();
    let vault = Vault::init(dir.path()).unwrap();
    let path = dir.path().join(NOTE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    std::fs::write(&path, "title = \"x\"\nestimate = 1 + 2\n").unwrap();
    match vault.read_wcl(NOTE) {
        Err(Error::DataOnly(e)) => assert_eq!(e.item, "estimate"),
        other => panic!("expected a data-only error, got {other:?}"),
    }

    std::fs::write(&path, "import \"./other.wcl\"\ntitle = \"x\"\n").unwrap();
    let env = wcl_lang::Environment::new();
    match vault.open_document(NOTE, &env) {
        Err(Error::DataOnly(e)) => assert_eq!(e.item, "`import \"./other.wcl\"`"),
        Err(other) => panic!("expected a data-only error, got {other:?}"),
        Ok(_) => panic!("expected a data-only error, got a document"),
    }
}
