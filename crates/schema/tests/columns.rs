//! `columns` layout blocks: their shape in content, in canvas cards and in template tabs.

use std::collections::HashMap;

use schema::{Issue, Schema, TabItem, TypeDef};

const TYPE: &str = r#"
label = "Cols"

field body {
  kind = "content"
}
field plan {
  kind = "canvas"
}
field status {
  kind = "string"
}
field due {
  kind = "date"
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
    let schema = Schema::from_sources(&[("cols", TYPE)], &[]).unwrap_or_else(|e| panic!("{e}"));
    let source = format!("title = \"A note\"\n{body}\n");
    schema
        .validate_note_source("cols", &source, "note.wcl", &HashMap::new())
        .map(|r| assert!(!r.is_flagged(), "{body}: {r:?}"))
        .map_err(|e| text(&e.issues))
}

fn column(width: &str, blocks: &str) -> String {
    let width = if width.is_empty() {
        String::new()
    } else {
        format!("    width = {width}\n")
    };
    format!("  column {{\n{width}{blocks}  }}\n")
}

fn para(t: &str) -> String {
    format!("    paragraph {{\n      text = \"{t}\"\n    }}\n")
}

fn content(columns: &[String]) -> String {
    format!("content body {{\n columns {{\n{}  }}\n}}", columns.concat())
}

fn has(issues: &[String], part: &str) -> bool {
    issues.iter().any(|i| i.contains(part))
}

#[test]
fn two_to_six_columns_with_positive_widths_and_content_blocks() {
    // Mixed blocks, an int and a float width, a column with no width and an empty column.
    let list = "    list {\n      style = \"todo\"\n      item {\n        text = \"x\"\n        checked = true\n      }\n    }\n";
    let code = "    code {\n      source = \"fn main() {}\"\n    }\n";
    check(&content(&[
        column("2", &format!("{}{list}", para("left"))),
        column("1.5", code),
        column("", ""),
    ]))
    .unwrap();
    for n in 2..=6 {
        let cols: Vec<String> = (0..n).map(|_| column("1", &para("x"))).collect();
        check(&content(&cols)).unwrap();
    }
}

#[test]
fn column_count_limits() {
    let one = check(&content(&[column("1", &para("alone"))])).unwrap_err();
    assert!(has(&one, "needs 2 to 6 columns, not 1"), "{one:?}");
    let seven: Vec<String> = (0..7).map(|_| column("1", &para("x"))).collect();
    let seven = check(&content(&seven)).unwrap_err();
    assert!(has(&seven, "needs 2 to 6 columns, not 7"), "{seven:?}");
    let none = check("content body {\n  columns {\n  }\n}").unwrap_err();
    assert!(has(&none, "not 0"), "{none:?}");
}

#[test]
fn widths_must_be_positive_numbers() {
    let zero = check(&content(&[column("0", ""), column("1", "")])).unwrap_err();
    assert!(
        has(&zero, "column#1.width: must be greater than 0"),
        "{zero:?}"
    );
    let negative = check(&content(&[column("1", ""), column("-2.5", "")])).unwrap_err();
    assert!(
        has(&negative, "column#2.width: must be greater than 0"),
        "{negative:?}"
    );
    let string = check(&content(&[column("\"wide\"", ""), column("1", "")])).unwrap_err();
    assert!(has(&string, "must be a number"), "{string:?}");
}

#[test]
fn children_are_checked_as_content_blocks() {
    let bad_heading = "    heading {\n      level = 9\n      text = \"x\"\n    }\n";
    let issues = check(&content(&[
        column("1", bad_heading),
        column("1", "    stroke {\n      tool = \"pen\"\n    }\n"),
    ]))
    .unwrap_err();
    assert!(
        has(
            &issues,
            "columns#1 > column#1 > heading#1.level: must be from 1 to 3"
        ),
        "{issues:?}"
    );
    assert!(
        has(&issues, "\"stroke\" is not a content block kind"),
        "{issues:?}"
    );

    // Only columns go in a columns block, a column takes no label and no other item, and a slot
    // is template-only.
    let shape = check(
        "content body {\n  columns {\n    paragraph {\n      text = \"x\"\n    }\n    column a {\n      gap = 1\n    }\n    column {\n      slot \"status\"\n    }\n  }\n}",
    )
    .unwrap_err();
    assert!(
        has(&shape, "columns#1 > paragraph: is not allowed here"),
        "{shape:?}"
    );
    assert!(has(&shape, "column#1: takes no label"), "{shape:?}");
    assert!(
        has(&shape, "column#1.gap: is not an item of this block"),
        "{shape:?}"
    );
    assert!(
        has(&shape, "\"slot\" is not a content block kind"),
        "{shape:?}"
    );
}

#[test]
fn a_column_cannot_hold_another_columns_block() {
    let inner = format!(
        "    columns {{\n    {}    {}    }}\n",
        column("1", ""),
        column("1", "")
    );
    let issues = check(&content(&[column("1", &inner), column("1", "")])).unwrap_err();
    assert!(
        has(
            &issues,
            "column#1 > columns#1: a column cannot hold another columns block"
        ),
        "{issues:?}"
    );
}

#[test]
fn a_card_holds_columns_but_not_a_canvas_inside_them() {
    let card = |inner: &str| {
        format!(
            "canvas plan {{\n  viewport {{\n    x = 0.0\n    y = 0.0\n    zoom = 1.0\n  }}\n  node c {{\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 200.0\n    height = 100.0\n    columns {{\n{}{}    }}\n  }}\n}}",
            column("1", &para("a")),
            column("1", inner)
        )
    };
    check(&card(&para("b"))).unwrap();
    let canvas = "    canvas {\n      viewport {\n        x = 0.0\n        y = 0.0\n        zoom = 1.0\n      }\n    }\n";
    let issues = check(&card(canvas)).unwrap_err();
    assert!(
        has(
            &issues,
            "columns#1 > column#2 > canvas#1: a card cannot hold a canvas"
        ),
        "{issues:?}"
    );
}

fn template(tab: &str) -> String {
    format!(
        "label = \"T\"\nfield status {{\n  kind = \"string\"\n}}\nfield due {{\n  kind = \"date\"\n}}\ntemplate default {{\n  tab \"Main\" {{\n{tab}  }}\n}}\n"
    )
}

fn type_issues(source: &str) -> Vec<String> {
    match TypeDef::parse("t", source) {
        Ok(t) => panic!("type parsed: {t:?}"),
        Err(e) => text(&e.issues),
    }
}

#[test]
fn template_slots_and_fixed_blocks_go_in_columns() {
    let tab = "    columns {\n      column {\n        width = 2\n        slot \"status\" {\n          label = \"State\"\n        }\n        paragraph {\n          text = \"Due {{due}}\"\n        }\n      }\n      column {\n        slot \"due\"\n      }\n    }\n    slot \"title\"\n";
    let t = TypeDef::parse("t", &template(tab)).unwrap_or_else(|e| panic!("{e}"));
    let items = &t.templates[0].tabs[0].items;
    assert_eq!(items.len(), 2);
    let TabItem::Block(cols) = &items[0] else {
        panic!("{items:?}")
    };
    assert_eq!(cols.kind, "columns");
    assert_eq!(cols.body.blocks_of("column").count(), 2);
}

#[test]
fn template_checks_walk_into_columns() {
    // A slot naming no field, a slot placed twice (once in a column, once in the tab) and a
    // `{{name}}` naming no field, all inside columns.
    let tab = "    slot \"status\"\n    columns {\n      column {\n        slot \"missing\"\n        slot \"status\"\n      }\n      column {\n        heading {\n          level = 2\n          text = \"About {{nothing}}\"\n        }\n      }\n    }\n";
    let issues = type_issues(&template(tab));
    assert!(
        has(
            &issues,
            "columns#2 > column#1 > slot#1: the type has no field \"missing\""
        ),
        "{issues:?}"
    );
    assert!(
        has(
            &issues,
            "column#1 > slot#2: places \"status\" a second time in this template"
        ),
        "{issues:?}"
    );
    assert!(
        has(&issues, "`{{nothing}}` names no field of the type"),
        "{issues:?}"
    );

    // The same field in two columns is placed twice, too, and a column holds no columns.
    let tab = "    columns {\n      column {\n        slot \"due\"\n      }\n      column {\n        slot \"due\"\n        columns {\n        }\n      }\n    }\n";
    let issues = type_issues(&template(tab));
    assert!(has(&issues, "a second time"), "{issues:?}");
    assert!(
        has(&issues, "a column cannot hold another columns block"),
        "{issues:?}"
    );
}
