//! `control` blocks: a form control bound to a field, checked in templates and in a note's own
//! content, at the top level, inside columns and inside canvas cards.

use std::collections::{BTreeMap, HashMap};

use schema::{CONTROL_KINDS, FieldChanges, Issue, Schema, TypeDef, Value};

const STATUS: &str = include_str!("../../../docs/vault-format/vault/taxonomies/status.wcl");

/// One field of every kind and cardinality a control can meet.
const FIELDS: &str = r#"
label = "Form"

field name {
  kind = "string"
}
field site {
  kind = "url"
}
field count {
  kind = "int"
}
field ratio {
  kind = "float"
}
field done {
  kind = "bool"
}
field due {
  kind = "date"
}
field at {
  kind = "datetime"
}
field opens {
  kind = "time"
}
field span {
  kind = "date_range"
}
field slot_span {
  kind = "date_range"
  of   = "datetime"
}
field size {
  kind    = "select"
  options = ["s", "m", "l"]
}
field sizes {
  kind     = "select"
  options  = ["s", "m", "l"]
  multiple = true
}
field status {
  kind     = "taxonomy"
  taxonomy = "status"
}
field statuses {
  kind     = "taxonomy"
  taxonomy = "status"
  multiple = true
}
field parent {
  kind   = "reference"
  target = "form"
}
field links {
  kind     = "reference"
  target   = "form"
  multiple = true
}
field tags {
  kind  = "list"
  items = "string"
}
field numbers {
  kind  = "list"
  items = "int"
}
field phones {
  kind = "list"
  item {
    kind = "dict"
    key number {
      kind     = "string"
      required = true
    }
    key label {
      kind = "string"
    }
  }
}
field rows {
  kind = "list"
  item {
    kind   = "dict"
    values = "string"
    key value {
      kind = "string"
    }
    key label {
      kind = "string"
    }
  }
}
field extra {
  kind   = "dict"
  values = "string"
}
field place {
  kind = "location"
  map  = "geo"
}
field body {
  kind = "content"
}
field cover {
  kind = "asset"
}
field sketch {
  kind = "ink"
}
field plan {
  kind = "canvas"
}
field words {
  kind     = "int"
  computed = true
}
"#;

/// The field each control kind fits, and every field it does not.
const FITS: &[(&str, &[&str])] = &[
    (
        "text",
        &["name", "site", "count", "ratio", "title", "words"],
    ),
    ("textarea", &["name", "title"]),
    ("number", &["count", "ratio", "words"]),
    ("slider", &["count", "ratio", "words"]),
    ("toggle", &["done"]),
    ("checkbox", &["done"]),
    ("dropdown", &["size", "status", "parent"]),
    ("radio", &["size"]),
    ("chips", &["tags", "phones", "sizes", "statuses", "links"]),
    ("calendar", &["due", "span"]),
    ("datetime", &["at"]),
    ("time", &["at", "opens"]),
    ("rating", &["count", "words"]),
];

const ALL: &[&str] = &[
    "title",
    "phones",
    "rows",
    "name",
    "site",
    "count",
    "ratio",
    "done",
    "due",
    "at",
    "opens",
    "span",
    "slot_span",
    "size",
    "sizes",
    "status",
    "statuses",
    "parent",
    "links",
    "tags",
    "numbers",
    "extra",
    "place",
    "body",
    "cover",
    "sketch",
    "plan",
    "words",
];

fn text(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

fn has(issues: &[String], part: &str) -> bool {
    issues.iter().any(|i| i.contains(part))
}

/// The items a control of `kind` needs to be valid on its own.
fn needed(kind: &str) -> &'static str {
    if kind == "slider" {
        "\n  min = 0\n  max = 10"
    } else {
        ""
    }
}

fn control(field: &str, kind: &str, extra: &str) -> String {
    format!(
        "control {{\n  field = \"{field}\"\n  kind = \"{kind}\"{}{extra}\n}}\n",
        needed(kind)
    )
}

/// A type whose default template tab holds `tab`.
fn type_with(tab: &str) -> Result<(), Vec<String>> {
    let source = format!("{FIELDS}\ntemplate default {{\n  tab \"Main\" {{\n{tab}  }}\n}}\n");
    let ty = TypeDef::parse("form", &source).map_err(|e| text(&e.issues))?;
    let tax = schema::Taxonomy::parse("status", STATUS).unwrap();
    Schema::new(vec![ty], vec![tax])
        .map_err(|e| e.0.iter().flat_map(|i| text(&i.issues)).collect::<Vec<_>>())?;
    Ok(())
}

fn schema() -> Schema {
    let source =
        format!("{FIELDS}\ntemplate default {{\n  tab \"Main\" {{\n    slot \"title\"\n  }}\n}}\n");
    Schema::from_sources(&[("form", &source)], &[("status", STATUS)])
        .unwrap_or_else(|e| panic!("{e}"))
}

/// Validate a note of type `form` with `body` after its title.
fn note(body: &str) -> Result<(), Vec<String>> {
    let source = format!("title = \"A note\"\n{body}\n");
    schema()
        .validate_note_source("form", &source, "note.wcl", &HashMap::new())
        .map(|r| assert!(!r.is_flagged(), "{body}: {r:?}"))
        .map_err(|e| text(&e.issues))
}

fn in_content(block: &str) -> String {
    format!("content body {{\n{block}}}")
}

fn in_columns(block: &str) -> String {
    format!(
        "columns {{\n  column {{\n    paragraph {{\n      text = \"left\"\n    }}\n  }}\n  column {{\n{block}  }}\n}}\n"
    )
}

#[test]
fn the_fit_table_covers_every_control_kind() {
    let kinds: Vec<&str> = FITS.iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, CONTROL_KINDS);
}

#[test]
fn each_control_kind_binds_the_fields_it_fits_in_a_template() {
    for (kind, fits) in FITS {
        for field in ALL {
            let result = type_with(&control(field, kind, ""));
            if fits.contains(field) {
                result.unwrap_or_else(|e| panic!("{kind} on {field}: {e:?}"));
            } else {
                let issues = result.expect_err(&format!("{kind} must not bind {field}"));
                assert!(
                    has(&issues, &format!("control#1.field: a {kind} control fits")),
                    "{kind} on {field}: {issues:?}"
                );
            }
        }
    }
}

#[test]
fn each_control_kind_binds_the_fields_it_fits_in_note_content() {
    for (kind, fits) in FITS {
        for field in ALL {
            let result = note(&in_content(&control(field, kind, "")));
            if fits.contains(field) {
                result.unwrap_or_else(|e| panic!("{kind} on {field}: {e:?}"));
            } else {
                let issues = result.expect_err(&format!("{kind} must not bind {field}"));
                assert!(
                    has(&issues, "content \"body\" > control#1.field: a"),
                    "{kind} on {field}: {issues:?}"
                );
            }
        }
    }
}

#[test]
fn the_reason_names_the_control_and_the_field() {
    let issues = type_with(&control("sizes", "dropdown", "")).unwrap_err();
    assert!(
        has(
            &issues,
            "template default > tab \"Main\" > control#1.field: a dropdown control fits a single select, taxonomy or reference field, and \"sizes\" is a list of select field"
        ),
        "{issues:?}"
    );
    let issues = type_with(&control("numbers", "chips", "")).unwrap_err();
    assert!(
        has(&issues, "\"numbers\" is a list of int field"),
        "{issues:?}"
    );
    let issues = type_with(&control("slot_span", "calendar", "")).unwrap_err();
    assert!(
        has(&issues, "is a date range of datetimes field"),
        "{issues:?}"
    );
}

#[test]
fn an_unknown_field_is_refused() {
    let issues = type_with(&control("nope", "text", "")).unwrap_err();
    assert!(
        has(&issues, "control#1.field: the type has no field \"nope\""),
        "{issues:?}"
    );
    let issues = note(&in_content(&control("nope", "text", ""))).unwrap_err();
    assert!(
        has(
            &issues,
            "content \"body\" > control#1.field: the type has no field \"nope\""
        ),
        "{issues:?}"
    );
}

#[test]
fn field_and_kind_are_required_and_the_kind_is_closed() {
    let issues = type_with("control {\n  label = \"x\"\n}\n").unwrap_err();
    assert!(has(&issues, "control#1.field: is required"), "{issues:?}");
    assert!(has(&issues, "control#1.kind: is required"), "{issues:?}");
    let issues = type_with(&control("name", "spinner", "")).unwrap_err();
    assert!(
        has(&issues, "\"spinner\" is not one of text, textarea"),
        "{issues:?}"
    );
    let issues = type_with("control a {\n  field = \"name\"\n  kind = \"text\"\n}\n").unwrap_err();
    assert!(has(&issues, "takes no label"), "{issues:?}");
    let issues = type_with("control {\n  field = \"name\"\n  kind = \"text\"\n  item {\n  }\n}\n")
        .unwrap_err();
    assert!(
        has(&issues, "control#1 > item: is not allowed here"),
        "{issues:?}"
    );
}

#[test]
fn label_and_placeholder() {
    type_with(&control(
        "name",
        "text",
        "\n  label = \"Name\"\n  placeholder = \"Your name\"",
    ))
    .unwrap();
    type_with(&control(
        "size",
        "dropdown",
        "\n  placeholder = \"Pick one\"",
    ))
    .unwrap();
    type_with(&control("done", "toggle", "\n  label = \"Finished\"")).unwrap();
    let issues = type_with(&control("done", "toggle", "\n  placeholder = \"x\"")).unwrap_err();
    assert!(
        has(
            &issues,
            "control#1.placeholder: is not used by a toggle control"
        ),
        "{issues:?}"
    );
    let issues = type_with(&control("name", "text", "\n  label = 3")).unwrap_err();
    assert!(
        has(&issues, "control#1.label: must be a string"),
        "{issues:?}"
    );
}

#[test]
fn slider_and_number_items() {
    type_with(&control("count", "slider", "\n  step = 2")).unwrap();
    type_with(&control("ratio", "slider", "\n  step = 0.5")).unwrap();
    type_with(&control(
        "count",
        "number",
        "\n  min = 1\n  max = 9\n  step = 1",
    ))
    .unwrap();
    type_with(&control("ratio", "number", "\n  min = -1.5")).unwrap();

    // A slider needs both ends.
    let issues = type_with("control {\n  field = \"count\"\n  kind = \"slider\"\n  max = 3\n}\n")
        .unwrap_err();
    assert!(
        has(&issues, "control#1.min: is required by a slider control"),
        "{issues:?}"
    );
    let issues = type_with("control {\n  field = \"count\"\n  kind = \"slider\"\n}\n").unwrap_err();
    assert!(
        has(&issues, "control#1.max: is required by a slider control"),
        "{issues:?}"
    );

    // min below max, a positive step, numbers only.
    let issues =
        type_with("control {\n  field = \"count\"\n  kind = \"slider\"\n  min = 5\n  max = 5\n}\n")
            .unwrap_err();
    assert!(
        has(&issues, "control#1.max: must be greater than min"),
        "{issues:?}"
    );
    let issues = type_with(&control("ratio", "number", "\n  step = 0")).unwrap_err();
    assert!(
        has(&issues, "control#1.step: must be greater than 0"),
        "{issues:?}"
    );
    let issues = type_with(&control("ratio", "number", "\n  min = \"low\"")).unwrap_err();
    assert!(
        has(&issues, "control#1.min: must be a number"),
        "{issues:?}"
    );

    // An int field takes whole numbers.
    let issues = type_with(&control("count", "slider", "\n  step = 0.5")).unwrap_err();
    assert!(
        has(
            &issues,
            "control#1.step: must be a whole number, as \"count\" is an int field"
        ),
        "{issues:?}"
    );

    // Other kinds take none of them.
    let issues = type_with(&control("name", "text", "\n  min = 1")).unwrap_err();
    assert!(
        has(&issues, "control#1.min: is not used by a text control"),
        "{issues:?}"
    );
    let issues = type_with(&control("count", "slider", "\n  placeholder = \"x\"")).unwrap_err();
    assert!(
        has(&issues, "is not used by a slider control"),
        "{issues:?}"
    );
}

#[test]
fn rating_items() {
    type_with(&control("count", "rating", "")).unwrap();
    type_with(&control("count", "rating", "\n  max = 10")).unwrap();
    type_with(&control("count", "rating", "\n  max = 1")).unwrap();
    for bad in ["0", "11", "-2"] {
        let issues =
            type_with(&control("count", "rating", &format!("\n  max = {bad}"))).unwrap_err();
        assert!(
            has(&issues, "control#1.max: must be from 1 to 10 stars"),
            "{bad}: {issues:?}"
        );
    }
    let issues = type_with(&control("count", "rating", "\n  max = 4.5")).unwrap_err();
    assert!(
        has(&issues, "control#1.max: must be a whole number of stars"),
        "{issues:?}"
    );
    let issues = type_with(&control("count", "rating", "\n  min = 1")).unwrap_err();
    assert!(
        has(&issues, "control#1.min: is not used by a rating control"),
        "{issues:?}"
    );
}

#[test]
fn computed_fields_may_be_bound_for_display() {
    type_with(&control("words", "number", "")).unwrap();
    note(&in_content(&control("words", "rating", ""))).unwrap();
}

#[test]
fn several_controls_may_bind_one_field_beside_its_slot() {
    let tab = format!(
        "    slot \"count\"\n{}{}",
        control("count", "number", ""),
        control("count", "slider", "")
    );
    type_with(&tab).unwrap();
}

#[test]
fn controls_inside_columns_are_checked() {
    type_with(&in_columns(&format!(
        "{}    slot \"name\"\n",
        control("done", "toggle", "")
    )))
    .unwrap();
    let issues = type_with(&in_columns(&control("name", "toggle", ""))).unwrap_err();
    assert!(
        has(
            &issues,
            "template default > tab \"Main\" > columns#1 > column#2 > control#1.field: a toggle control fits a bool field"
        ),
        "{issues:?}"
    );
    note(&in_content(&in_columns(&control("due", "calendar", "")))).unwrap();
    let issues = note(&in_content(&in_columns(&control("nope", "calendar", "")))).unwrap_err();
    assert!(
        has(
            &issues,
            "content \"body\" > columns#1 > column#2 > control#1.field: the type has no field \"nope\""
        ),
        "{issues:?}"
    );
    let issues = note(&in_content(&in_columns(&control(
        "count",
        "rating",
        "\n  max = 20",
    ))))
    .unwrap_err();
    assert!(
        has(&issues, "column#2 > control#1.max: must be from 1 to 10"),
        "{issues:?}"
    );
}

fn canvas_with_card(card: &str) -> String {
    format!(
        "canvas plan {{\n  viewport {{\n    x = 0\n    y = 0\n    zoom = 1\n  }}\n  node a {{\n    kind = \"card\"\n    x = 0\n    y = 0\n    width = 200\n    height = 100\n{card}  }}\n}}"
    )
}

#[test]
fn controls_in_canvas_cards_bind_the_holding_note() {
    note(&canvas_with_card(&control("done", "checkbox", ""))).unwrap();
    let issues = note(&canvas_with_card(&control("done", "radio", ""))).unwrap_err();
    assert!(
        has(
            &issues,
            "canvas \"plan\" > node \"a\" > control#1.field: a radio control fits a single select field"
        ),
        "{issues:?}"
    );
    // A canvas block in a template binds the type's fields too.
    let tab = format!(
        "    canvas {{\n      viewport {{\n        x = 0\n        y = 0\n        zoom = 1\n      }}\n      node a {{\n        kind = \"card\"\n        x = 0\n        y = 0\n        width = 1\n        height = 1\n{}      }}\n    }}\n",
        control("nope", "text", "")
    );
    let issues = type_with(&tab).unwrap_err();
    assert!(has(&issues, "the type has no field \"nope\""), "{issues:?}");
}

#[test]
fn a_renamed_field_takes_its_controls_with_it() {
    let tab = "    slot \"title\"\n";
    let old = format!("{FIELDS}\ntemplate default {{\n  tab \"Main\" {{\n{tab}  }}\n}}\n");
    let new = old.replace("field count {", "field total {");
    let (old, new) = (
        TypeDef::parse("form", &old).unwrap(),
        TypeDef::parse("form", &new).unwrap(),
    );
    let renames = BTreeMap::from([("count".to_owned(), "total".to_owned())]);
    let changes = FieldChanges::new(&old, &new, &renames).unwrap();
    let doc = schema::Doc::parse(
        &format!(
            "title = \"T\"\n{}",
            in_content(&in_columns(&control("count", "number", "")))
        ),
        "n.wcl",
    )
    .unwrap();
    let (after, edits) = changes.apply(&doc);
    assert_eq!(edits.len(), 1, "{edits:?}");
    let control = &after.blocks[0].body.blocks[0].body.blocks[1].body.blocks[0];
    assert_eq!(
        control.body.field("field"),
        Some(&Value::Str("total".into()))
    );
}
