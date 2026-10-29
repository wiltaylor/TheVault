//! `table` content blocks: an optional header flag and column alignments, then rows of inline
//! markup cells, in content, nested blocks, canvas cards and templates.

use std::collections::HashMap;

use schema::{Issue, Schema, TABLE_COLUMNS_MAX, TABLE_ROWS_MAX, TypeDef};

const TYPE: &str = r#"
label = "Doc"

field body {
  kind = "content"
}
field plan {
  kind = "canvas"
}

template default {
  tab "All" {
    slot "title"
  }
}
"#;

fn text(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

fn check(body: &str) -> Result<(), Vec<String>> {
    let schema = Schema::from_sources(&[("doc", TYPE)], &[]).unwrap_or_else(|e| panic!("{e}"));
    let source = format!("title = \"A note\"\ncontent body {{\n{body}\n}}\n");
    schema
        .validate_note_source("doc", &source, "note.wcl", &HashMap::new())
        .map(|r| assert!(!r.is_flagged(), "{body}: {r:?}"))
        .map_err(|e| text(&e.issues))
}

fn has(issues: &[String], part: &str) -> bool {
    issues.iter().any(|i| i.contains(part))
}

/// A table of `rows`, each a list of cells written as WCL, with `items` before them.
fn table(items: &str, rows: &[&str]) -> String {
    let rows: String = rows
        .iter()
        .map(|r| format!("  row {{\n    cells = [{r}]\n  }}\n"))
        .collect();
    format!("table {{\n{items}{rows}}}")
}

#[test]
fn a_table_holds_rows_of_markup_cells_with_an_optional_header_and_alignments() {
    check(&table("", &[r#""a", "b""#])).unwrap();
    check(&table(
        "  header = true\n  align = [\"left\", \"right\", \"center\"]\n",
        &[
            r#""Item", "Qty", "Notes""#,
            r#""Apples", "3", "**ripe**, see [[01JB8Z4K2MQ7V3X9T5R6W8Y2ZA|Groceries]]""#,
        ],
    ))
    .unwrap();
    // Rows may differ in length, and `align` may name fewer columns than there are, or more.
    check(&table(
        "  align = [\"right\"]\n",
        &[r#""a""#, r#""b", "c", "d""#, ""],
    ))
    .unwrap();
    check(&table(
        "  align = [\"center\", \"left\", \"right\"]\n",
        &[r#""a""#],
    ))
    .unwrap();
    // A table sits in toggles, admonitions, columns and canvas cards as other blocks do.
    let t = table("", &[r#""x""#]);
    check(&format!("toggle {{\n  text = \"t\"\n{t}\n}}")).unwrap();
    check(&format!("admonition {{\n  kind = \"tip\"\n{t}\n}}")).unwrap();
    check(&format!(
        "columns {{\n  column {{\n{t}\n  }}\n  column {{\n{t}\n  }}\n}}"
    ))
    .unwrap();
}

#[test]
fn a_table_needs_a_row_and_a_cell_and_rows_of_string_cells() {
    let none = check("table {\n  header = true\n}").unwrap_err();
    assert!(has(&none, "table#1: needs at least one row"), "{none:?}");
    let empty = check(&table("", &["", ""])).unwrap_err();
    assert!(has(&empty, "table#1: needs at least one cell"), "{empty:?}");
    let missing = check("table {\n  row {\n  }\n}").unwrap_err();
    assert!(
        has(&missing, "table#1 > row#1.cells: is required"),
        "{missing:?}"
    );
    let number = check(&table("", &[r#""a", 2"#])).unwrap_err();
    assert!(
        has(&number, "table#1 > row#1.cells: must be a list of strings"),
        "{number:?}"
    );
    let odd =
        check("table {\n  caption = \"x\"\n  row {\n    cells = [\"a\"]\n  }\n}").unwrap_err();
    assert!(
        has(&odd, "table#1.caption: is not an item of this block"),
        "{odd:?}"
    );
    let block = check("table {\n  paragraph {\n    text = \"x\"\n  }\n}").unwrap_err();
    assert!(
        has(&block, "table#1 > paragraph: is not allowed here"),
        "{block:?}"
    );
    let labelled = check("table {\n  row \"r\" {\n    cells = [\"a\"]\n  }\n}").unwrap_err();
    assert!(has(&labelled, "row#1: takes no label"), "{labelled:?}");
    let header = check(&table("  header = \"yes\"\n", &[r#""a""#])).unwrap_err();
    assert!(
        has(&header, "table#1.header: must be true or false"),
        "{header:?}"
    );
}

#[test]
fn alignments_are_left_center_or_right() {
    let bad = check(&table(
        "  align = [\"left\", \"middle\"]\n",
        &[r#""a", "b""#],
    ))
    .unwrap_err();
    assert!(
        has(
            &bad,
            "table#1.align[1]: \"middle\" is not one of left, center, right"
        ),
        "{bad:?}"
    );
    let not_list = check(&table("  align = \"left\"\n", &[r#""a""#])).unwrap_err();
    assert!(
        has(&not_list, "table#1.align: must be a list of strings"),
        "{not_list:?}"
    );
}

#[test]
fn a_table_has_at_most_100_columns_and_2000_rows() {
    let cells = |n: usize| vec!["\"c\""; n].join(", ");
    check(&table("", &[&cells(TABLE_COLUMNS_MAX)])).unwrap();
    let wide = check(&table("", &[&cells(TABLE_COLUMNS_MAX + 1)])).unwrap_err();
    assert!(
        has(
            &wide,
            "row#1.cells: has 101 cells; a table has at most 100 columns"
        ),
        "{wide:?}"
    );
    let row = "\"r\"".to_owned();
    let rows = vec![row.as_str(); TABLE_ROWS_MAX + 1];
    let long = check(&table("", &rows)).unwrap_err();
    assert!(
        has(&long, "table#1: has 2001 rows; a table holds at most 2000"),
        "{long:?}"
    );
    check(&table("", &rows[..TABLE_ROWS_MAX])).unwrap();
}

#[test]
fn placeholders_in_cells_are_checked() {
    let issues = check(&table("", &[r#""fine", "[[+nosuch|A]]""#])).unwrap_err();
    assert!(
        has(
            &issues,
            "table#1 > row#1.cells[1]: the placeholder link titled \"A\" names type \"nosuch\""
        ),
        "{issues:?}"
    );
    check(&table("", &[r#""[[+doc|A new doc]]""#])).unwrap();
}

#[test]
fn a_template_table_may_show_fields_in_its_cells() {
    let tab = |cell: &str| {
        format!(
            "label = \"T\"\nfield due {{\n  kind = \"date\"\n}}\ntemplate default {{\n  tab \"Main\" {{\n    table {{\n      row {{\n        cells = [\"Due\", \"{cell}\"]\n      }}\n    }}\n  }}\n}}\n"
        )
    };
    TypeDef::parse("t", &tab("{{due}}")).unwrap_or_else(|e| panic!("{e}"));
    let issues = match TypeDef::parse("t", &tab("{{nothing}}")) {
        Ok(t) => panic!("type parsed: {t:?}"),
        Err(e) => text(&e.issues),
    };
    assert!(
        has(&issues, "row.cells[1]: `{{nothing}}` names no field"),
        "{issues:?}"
    );
}

// rich-import H5: a divider is a block with nothing in it, wherever content blocks sit.
#[test]
fn a_divider_holds_nothing_and_sits_wherever_content_does() {
    check("divider").unwrap();
    check("toggle {\n  text = \"t\"\n  divider\n}").unwrap();
    check("columns {\n  column {\n    divider\n  }\n  column {\n    divider\n  }\n}").unwrap();
    let item = check("divider {\n  style = \"dashed\"\n}").unwrap_err();
    assert!(
        has(&item, "divider#1.style: is not an item of this block"),
        "{item:?}"
    );
    let block = check("divider {\n  paragraph {\n    text = \"x\"\n  }\n}").unwrap_err();
    assert!(
        has(&block, "divider#1 > paragraph: is not allowed here"),
        "{block:?}"
    );
}
