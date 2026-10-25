//! Schema parsing and note validation, at the crate's public seam.

use std::collections::HashMap;

use schema::{FieldKind, Issue, NoteLookup, OnDelete, RangeOf, Relation, Schema, TypeDef, Value};

const SPECIMEN_TYPE: &str = include_str!("../../../docs/vault-format/vault/types/specimen.wcl");
const BOARD_TYPE: &str = include_str!("../../../docs/vault-format/vault/types/board.wcl");
const STATUS: &str = include_str!("../../../docs/vault-format/vault/taxonomies/status.wcl");
const TOPIC: &str = include_str!("../../../docs/vault-format/vault/taxonomies/topic.wcl");
const SPECIMEN_NOTE: &str =
    include_str!("../../../docs/vault-format/vault/notes/specimen/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA.wcl");
const BOARD_NOTE: &str =
    include_str!("../../../docs/vault-format/vault/notes/board/01JB8Z4K2MQ7V3X9T5R6W8Y2ZF.wcl");

const BOX_NOTE: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const KINDS_NOTE: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZC";
const MISSING_NOTE: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ";
const ASSET: &str = "01JB9A0C3D4E5F6G7H8J9K0M1N";

fn example() -> Schema {
    Schema::from_sources(
        &[("specimen", SPECIMEN_TYPE), ("board", BOARD_TYPE)],
        &[("status", STATUS), ("topic", TOPIC)],
    )
    .unwrap()
}

fn lookup(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(id, t)| (id.to_string(), t.to_string()))
        .collect()
}

/// Every issue as `at: message`, for assertions and failure output.
fn text(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

#[test]
fn the_example_vault_is_valid() {
    let schema = example();
    let specimen = schema.type_def("specimen").unwrap();
    assert_eq!(specimen.fields.len(), 29);
    assert_eq!(specimen.plural, "Specimens");
    assert!(specimen.field("word_count").unwrap().computed);
    let parent = specimen.field("parent").unwrap().reference().unwrap();
    assert_eq!(parent.relation, Relation::TwoWay);
    assert_eq!(parent.on_delete, OnDelete::Cascade);
    assert_eq!(parent.inverse.as_deref(), Some("children"));
    assert_eq!(
        specimen
            .field("children")
            .unwrap()
            .reference()
            .unwrap()
            .on_delete,
        OnDelete::Unlink
    );
    assert_eq!(specimen.handlers[0].function, "count_words");
    assert_eq!(specimen.templates.len(), 2);

    let notes = lookup(&[
        ("01JB8Z4K2MQ7V3X9T5R6W8Y2ZA", "specimen"),
        ("01JB8Z4K2MQ7V3X9T5R6W8Y2ZB", "specimen"),
        ("01JB8Z4K2MQ7V3X9T5R6W8Y2ZC", "specimen"),
        ("01JB8Z4K2MQ7V3X9T5R6W8Y2ZD", "specimen"),
        ("01JB8Z4K2MQ7V3X9T5R6W8Y2ZE", "specimen"),
        ("01JB8Z4K2MQ7V3X9T5R6W8Y2ZF", "board"),
    ]);
    let report = schema
        .validate_note_source("specimen", SPECIMEN_NOTE, "specimen.wcl", &notes)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!report.is_flagged(), "{report:?}");
    let report = schema
        .validate_note_source("board", BOARD_NOTE, "board.wcl", &notes)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!report.is_flagged(), "{report:?}");
}

#[test]
fn kanban_eligibility_is_single_term_taxonomy_fields() {
    let schema = example();
    assert!(schema.is_kanban_group("specimen", "status"));
    assert!(!schema.is_kanban_group("specimen", "topics"));
    assert!(!schema.is_kanban_group("specimen", "priority"));
    assert!(!schema.is_kanban_group("specimen", "missing"));
    let names: Vec<&str> = schema
        .type_def("specimen")
        .unwrap()
        .kanban_fields()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["status"]);
}

#[test]
fn templates_fall_back_as_the_format_says() {
    let schema = example();
    let board = schema.type_def("board").unwrap();
    assert_eq!(board.resolve_template("card").unwrap().tabs.len(), 1);
    assert!(board.resolve_template("detail").is_none());
    assert!(board.resolve_template("compact").is_none());
}

/// A type with one optional field of every kind, and a second type to reference.
const KINDS_TYPE: &str = r#"
label = "Kinds"

field s {
  kind = "string"
}
field i {
  kind = "int"
}
field f {
  kind = "float"
}
field b {
  kind = "bool"
}
field d {
  kind = "date"
}
field dt {
  kind = "datetime"
}
field dr {
  kind = "date_range"
}
field drt {
  kind = "date_range"
  of   = "datetime"
}
field dre {
  kind = "date_range"
  of   = "date_or_datetime"
}
field u {
  kind = "url"
}
field sel {
  kind    = "select"
  options = ["low", "high"]
}
field sels {
  kind     = "select"
  options  = ["email", "chat"]
  multiple = true
}
field tax {
  kind     = "taxonomy"
  taxonomy = "status"
}
field taxes {
  kind     = "taxonomy"
  taxonomy = "status"
  multiple = true
}
field r {
  kind   = "reference"
  target = "box"
}
field rs {
  kind     = "reference"
  target   = "box"
  multiple = true
}
field anyref {
  kind   = "reference"
  target = "*"
}
field l {
  kind  = "list"
  items = "date"
}
field dc {
  kind   = "dict"
  values = "int"
}
field geo {
  kind = "location"
  map  = "geo"
}
field pin {
  kind  = "location"
  map   = "image"
  image = "01JB9A0C3D4E5F6G7H8J9K0M1Q"
}
field body {
  kind = "content"
}
field a {
  kind = "asset"
}
field files {
  kind     = "asset"
  multiple = true
}
field sketch {
  kind = "ink"
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

const BOX_TYPE: &str = r#"
label = "Box"

template default {
  tab "Box" {
    slot "title"
  }
}
"#;

fn kinds() -> Schema {
    Schema::from_sources(
        &[("kinds", KINDS_TYPE), ("box", BOX_TYPE)],
        &[("status", STATUS)],
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

fn check_kinds(body: &str) -> Result<(), Vec<String>> {
    let notes = lookup(&[(BOX_NOTE, "box"), (KINDS_NOTE, "kinds")]);
    let source = format!("title = \"A note\"\n{body}\n");
    kinds()
        .validate_note_source("kinds", &source, "note.wcl", &notes)
        .map(|r| assert!(!r.is_flagged(), "{body}: {r:?}"))
        .map_err(|e| text(&e.issues))
}

#[test]
fn every_field_kind_accepts_a_valid_value_and_rejects_an_invalid_one() {
    // (kind, field, valid, invalid). The field the invalid value is written to must be named in
    // at least one issue.
    let ink_ok = "ink sketch {\n width = 10\n height = 10\n stroke {\n tool = \"pen\"\n colour = \"#1f2937\"\n size = 2.0\n points = [1.0, 2.0, 0.5]\n }\n}";
    let ink_bad = "ink sketch {\n width = 10\n height = 10\n stroke {\n tool = \"pen\"\n colour = \"#1F29ZZ\"\n size = 2.0\n points = [1.0, 2.0, 0.5]\n }\n}";
    let canvas_ok = "canvas plan {\n viewport {\n x = 0.0\n y = 0.0\n zoom = 1.0\n }\n node a {\n kind = \"card\"\n x = 0.0\n y = 0.0\n width = 10.0\n height = 10.0\n paragraph {\n text = \"hi\"\n }\n }\n edge e {\n from = \"a\"\n to = \"a\"\n }\n}";
    let canvas_bad = "canvas plan {\n viewport {\n x = 0.0\n y = 0.0\n zoom = 1.0\n }\n edge e {\n from = \"a\"\n to = \"b\"\n }\n}";
    let rows: Vec<(&str, &str, String, String)> = vec![
        ("string", "s", r#"s = "hi""#.into(), "s = 3".into()),
        ("int", "i", "i = -3".into(), "i = 3.5".into()),
        ("float", "f", "f = 3".into(), r#"f = "3.0""#.into()),
        ("bool", "b", "b = false".into(), r#"b = "false""#.into()),
        ("date", "d", r#"d = "2024-02-29""#.into(), r#"d = "2026-02-29""#.into()),
        (
            "datetime",
            "dt",
            r#"dt = "2026-10-20T09:30:00+11:00""#.into(),
            r#"dt = "2026-10-20T09:30""#.into(),
        ),
        (
            "date_range",
            "dr",
            r#"dr = { start: "2026-10-20", end: "2026-11-01" }"#.into(),
            r#"dr = { start: "2026-11-01", end: "2026-10-20" }"#.into(),
        ),
        (
            "date_range",
            "dr",
            r#"dr = { end: "2026-10-20", start: "2026-10-20" }"#.into(),
            r#"dr = { start: "2026-10-20" }"#.into(),
        ),
        (
            "date_range (datetime)",
            "drt",
            r#"drt = { start: "2026-10-20T09:00:00+11:00", end: "2026-10-19T23:00:00Z" }"#.into(),
            r#"drt = { start: "2026-10-20", end: "2026-10-21" }"#.into(),
        ),
        ("url", "u", r#"u = "https://example.com/x""#.into(), r#"u = "example.com""#.into()),
        ("select", "sel", r#"sel = "high""#.into(), r#"sel = "urgent""#.into()),
        (
            "select (multiple)",
            "sels",
            r#"sels = ["email", "chat"]"#.into(),
            r#"sels = "email""#.into(),
        ),
        ("taxonomy", "tax", r#"tax = "Active""#.into(), r#"tax = "Paused""#.into()),
        (
            "taxonomy (multiple)",
            "taxes",
            r#"taxes = ["Inbox", "Active/Blocked"]"#.into(),
            r#"taxes = ["Inbox", "Inbox"]"#.into(),
        ),
        ("reference", "r", format!("r = {BOX_NOTE:?}"), r#"r = "01jb8z4k""#.into()),
        (
            "reference (multiple)",
            "rs",
            format!("rs = [{BOX_NOTE:?}]"),
            format!("rs = {BOX_NOTE:?}"),
        ),
        ("reference (any type)", "anyref", format!("anyref = {KINDS_NOTE:?}"), "anyref = 7".into()),
        (
            "list",
            "l",
            r#"l = ["2026-01-01", "2026-01-02"]"#.into(),
            r#"l = ["2026-01-01", 2]"#.into(),
        ),
        (
            "dict",
            "dc",
            "dict dc {\n  a = 1\n  b = 2\n}".into(),
            "dict dc {\n  a = \"one\"\n}".into(),
        ),
        (
            "location (geo)",
            "geo",
            "geo = { lat: -37.8136, lng: 144.9631 }".into(),
            "geo = { lat: 95.0, lng: 0.0 }".into(),
        ),
        (
            "location (image)",
            "pin",
            "pin = { x: 412.0, y: 188.5 }".into(),
            "pin = { lat: 1.0, lng: 2.0 }".into(),
        ),
        (
            "content",
            "body",
            "content body {\n  heading {\n    level = 2\n    text = \"Plan\"\n  }\n}".into(),
            "content body {\n  heading {\n    level = 4\n    text = \"Plan\"\n  }\n}".into(),
        ),
        (
            "content",
            "body",
            "content body {\n  list {\n    style = \"todo\"\n    item {\n      text = \"a\"\n      checked = true\n    }\n  }\n}".into(),
            "content body {\n  table {\n    text = \"no such block\"\n  }\n}".into(),
        ),
        ("asset", "a", format!("a = {ASSET:?}"), r#"a = "cover.png""#.into()),
        ("asset (multiple)", "files", format!("files = [{ASSET:?}]"), "files = [1]".into()),
        ("ink", "sketch", ink_ok.into(), ink_bad.into()),
        ("canvas", "plan", canvas_ok.into(), canvas_bad.into()),
    ];

    let covered: std::collections::HashSet<&str> = rows
        .iter()
        .map(|r| r.0.split(' ').next().unwrap())
        .collect();
    for kind in FieldKind::NAMES {
        assert!(covered.contains(kind), "no row for field kind {kind}");
    }

    for (kind, field, valid, invalid) in &rows {
        if let Err(issues) = check_kinds(valid) {
            panic!("{kind}: valid value `{valid}` was rejected: {issues:?}");
        }
        match check_kinds(invalid) {
            Ok(()) => panic!("{kind}: invalid value `{invalid}` was accepted"),
            Err(issues) => assert!(
                issues.iter().any(|i| i.contains(field)),
                "{kind}: issues for `{invalid}` do not name {field}: {issues:?}"
            ),
        }
    }
}

/// `of = "date_or_datetime"`: both ends dates or both datetimes, never one of each, and the end
/// never before the start (google-sync D15).
#[test]
fn a_date_or_datetime_range_takes_either_form_but_not_both() {
    let schema = kinds();
    let dre = schema.type_def("kinds").unwrap().field("dre").unwrap();
    assert_eq!(
        dre.kind,
        FieldKind::DateRange {
            of: RangeOf::DateOrDatetime
        }
    );
    for ok in [
        r#"dre = { start: "2026-10-20", end: "2026-10-20" }"#,
        r#"dre = { start: "2026-10-20", end: "2026-10-22" }"#,
        r#"dre = { start: "2026-10-20T09:00:00+11:00", end: "2026-10-19T23:00:00Z" }"#,
        r#"dre = { start: "2026-10-20T09:00:00Z", end: "2026-10-20T09:00:00Z" }"#,
    ] {
        if let Err(issues) = check_kinds(ok) {
            panic!("`{ok}` was rejected: {issues:?}");
        }
    }
    for (bad, want) in [
        (
            r#"dre = { start: "2026-10-20", end: "2026-10-20T10:00:00Z" }"#,
            "dre.end: must be a date, as start is",
        ),
        (
            r#"dre = { start: "2026-10-20T10:00:00Z", end: "2026-10-21" }"#,
            "dre.end: must be a datetime, as start is",
        ),
        (
            r#"dre = { start: "2026-10-21", end: "2026-10-20" }"#,
            "dre.end: is before start",
        ),
        (
            r#"dre = { start: "2026-10-20T10:00:00Z", end: "2026-10-20T09:59:59Z" }"#,
            "dre.end: is before start",
        ),
        (
            r#"dre = { start: "2026-10-20T10:00", end: "2026-10-21" }"#,
            "dre.start: must be an ISO 8601 date or datetime",
        ),
        (r#"dre = { start: "2026-10-20" }"#, "dre"),
    ] {
        let issues = check_kinds(bad).expect_err(bad);
        assert!(
            issues.iter().any(|i| i.starts_with(want)),
            "`{bad}`: wanted `{want}`, got {issues:?}"
        );
    }
    // The other two forms still refuse what they always refused.
    assert!(
        check_kinds(r#"dr = { start: "2026-10-20T10:00:00Z", end: "2026-10-20T11:00:00Z" }"#)
            .is_err()
    );
    assert!(check_kinds(r#"drt = { start: "2026-10-20", end: "2026-10-21" }"#).is_err());
}

#[test]
fn an_unknown_date_range_form_is_refused() {
    let ty = "label = \"Bad\"\nfield when {\n  kind = \"date_range\"\n  of   = \"either\"\n}\n\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n  }\n}\n";
    let err = Schema::from_sources(&[("bad", ty)], &[]).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("date_or_datetime"), "{text}");
}

#[test]
fn a_reference_to_a_missing_note_fails() {
    let issues = check_kinds(&format!("r = {MISSING_NOTE:?}")).unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.starts_with("r:") && i.contains("does not exist")),
        "{issues:?}"
    );

    let issues = check_kinds(&format!("rs = [{BOX_NOTE:?}, {MISSING_NOTE:?}]")).unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.starts_with("rs[1]:") && i.contains("does not exist")),
        "{issues:?}"
    );

    let issues = check_kinds(&format!("anyref = {MISSING_NOTE:?}")).unwrap_err();
    assert!(
        issues.iter().any(|i| i.contains("does not exist")),
        "{issues:?}"
    );
}

#[test]
fn a_reference_to_a_note_of_the_wrong_type_fails() {
    let issues = check_kinds(&format!("r = {KINDS_NOTE:?}")).unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.starts_with("r:") && i.contains("is a kinds, but this field holds a box")),
        "{issues:?}"
    );
    // `"*"` accepts a note of any type.
    check_kinds(&format!("anyref = {BOX_NOTE:?}")).unwrap();
}

#[test]
fn a_term_must_exist_in_the_taxonomy() {
    // A top-level term and a term deep in the hierarchy pass.
    check_kinds(r#"tax = "Inbox""#).unwrap();
    check_kinds(r#"tax = "Active/In progress""#).unwrap();

    for missing in [
        "Archived",
        "Active/Paused",
        "In progress",
        "Active/In progress/Deeper",
    ] {
        let issues = check_kinds(&format!("tax = {missing:?}")).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("is not a term of taxonomy \"status\"")),
            "{missing}: {issues:?}"
        );
    }
    let issues = check_kinds(r#"taxes = ["Inbox", "Nope"]"#).unwrap_err();
    assert!(
        issues.iter().any(|i| i.starts_with("taxes[1]:")),
        "{issues:?}"
    );
}

#[test]
fn a_deep_term_passes_in_a_deeper_hierarchy() {
    let tax = "label = \"Place\"\nterm \"Earth\" {\n  term \"Europe\" {\n    term \"France\" {\n      term \"Paris\"\n    }\n  }\n}\n";
    let ty = "label = \"Trip\"\nfield where {\n  kind = \"taxonomy\"\n  taxonomy = \"place\"\n}\ntemplate default {\n  tab \"T\" {\n    slot \"where\"\n  }\n}\n";
    let schema = Schema::from_sources(&[("trip", ty)], &[("place", tax)]).unwrap();
    let none = lookup(&[]);
    schema
        .validate_note_source(
            "trip",
            "title = \"t\"\nwhere = \"Earth/Europe/France/Paris\"\n",
            "n",
            &none,
        )
        .unwrap();
    let err = schema
        .validate_note_source(
            "trip",
            "title = \"t\"\nwhere = \"Earth/Europe/Paris\"\n",
            "n",
            &none,
        )
        .unwrap_err();
    assert!(err.to_string().contains("not a term"), "{err}");
}

fn type_issues(source: &str) -> Vec<String> {
    match TypeDef::parse("t", source) {
        Ok(t) => panic!("type parsed: {t:?}"),
        Err(e) => text(&e.issues),
    }
}

#[test]
fn a_template_naming_a_field_the_type_lacks_fails() {
    let issues = type_issues(
        "label = \"T\"\nfield a {\n  kind = \"string\"\n}\ntemplate default {\n  tab \"Main\" {\n    slot \"title\"\n    slot \"a\"\n  }\n}\ntemplate card {\n  tab \"Card\" {\n    slot \"missing\"\n  }\n}\n",
    );
    assert!(
        issues
            .iter()
            .any(|i| i.starts_with("template card")
                && i.contains("the type has no field \"missing\"")),
        "{issues:?}"
    );
}

#[test]
fn a_field_reference_in_template_text_must_name_a_field() {
    let ok = "label = \"T\"\nfield a {\n  kind = \"string\"\n}\ntemplate default {\n  tab \"Main\" {\n    paragraph {\n      text = \"{{title}} is {{a}}, not \\\\{{b}} or `{{c}}` or {{ d }}\"\n    }\n  }\n}\n";
    TypeDef::parse("t", ok).expect("known names, escaped braces and code spans are fine");

    let issues = type_issues(
        "label = \"T\"\nfield a {\n  kind = \"string\"\n}\ntemplate default {\n  tab \"Main\" {\n    heading {\n      level = 2\n      text = \"About {{a}}\"\n    }\n    list {\n      style = \"bullet\"\n      item {\n        text = \"Due {{due}}\"\n      }\n    }\n  }\n}\n",
    );
    assert!(
        issues.iter().any(|i| i.starts_with("template default")
            && i.contains("`{{due}}` names no field of the type")),
        "{issues:?}"
    );
    assert_eq!(issues.len(), 1, "{issues:?}");
}

#[test]
fn template_rules() {
    let field_twice = type_issues(
        "label = \"T\"\nfield a {\n  kind = \"string\"\n}\ntemplate default {\n  tab \"One\" {\n    slot \"a\"\n  }\n  tab \"Two\" {\n    slot \"a\"\n  }\n}\n",
    );
    assert!(
        field_twice.iter().any(|i| i.contains("a second time")),
        "{field_twice:?}"
    );

    let no_default =
        type_issues("label = \"T\"\ntemplate card {\n  tab \"C\" {\n    slot \"title\"\n  }\n}\n");
    assert!(
        no_default.iter().any(|i| i.contains("`default` template")),
        "{no_default:?}"
    );

    let no_tab = type_issues("label = \"T\"\ntemplate default {\n  label = \"x\"\n}\n");
    assert!(
        no_tab.iter().any(|i| i.contains("at least one tab")),
        "{no_tab:?}"
    );

    let bad_block = type_issues(
        "label = \"T\"\ntemplate default {\n  tab \"T\" {\n    heading {\n      level = 9\n      text = \"x\"\n    }\n  }\n}\n",
    );
    assert!(
        bad_block.iter().any(|i| i.contains("level")),
        "{bad_block:?}"
    );
}

#[test]
fn field_definition_rules() {
    let def = |field: &str| {
        type_issues(&format!(
            "label = \"T\"\n{field}\ntemplate default {{\n  tab \"T\" {{\n    slot \"title\"\n  }}\n}}\n"
        ))
    };
    let cases: &[(&str, &str)] = &[
        ("field a {\n  kind = \"colour\"\n}", "is not one of"),
        ("field title {\n  kind = \"string\"\n}", "reserved"),
        ("field archived {\n  kind = \"bool\"\n}", "reserved"),
        (
            "field Bad {\n  kind = \"string\"\n}",
            "not a valid field name",
        ),
        (
            "field a {\n  kind = \"int\"\n  required = true\n  computed = true\n}",
            "computed field cannot also be required",
        ),
        (
            "field a {\n  kind = \"int\"\n  required = true\n  hidden = true\n}",
            "hidden field cannot also be required",
        ),
        (
            "field a {\n  kind = \"int\"\n  hidden = \"yes\"\n}",
            "hidden",
        ),
        (
            "field a {\n  kind = \"int\"\n  multiple = true\n}",
            "not allowed on a int field",
        ),
        (
            "field a {\n  kind = \"int\"\n  default = \"three\"\n}",
            "must be a number",
        ),
        (
            "field a {\n  kind = \"content\"\n  default = \"x\"\n}",
            "not allowed on a content field",
        ),
        (
            "field a {\n  kind = \"select\"\n  options = []\n}",
            "at least one option",
        ),
        (
            "field a {\n  kind = \"select\"\n  options = [\"x\"]\n  default = \"y\"\n}",
            "not one of the options",
        ),
        (
            "field a {\n  kind = \"list\"\n  items = \"canvas\"\n}",
            "is not one of",
        ),
        (
            "field a {\n  kind = \"location\"\n  map = \"image\"\n}",
            "required when map is \"image\"",
        ),
        (
            "field a {\n  kind = \"reference\"\n  target = \"t\"\n  relation = \"two_way\"\n}",
            "required for a two_way",
        ),
        (
            "field a {\n  kind = \"reference\"\n  target = \"t\"\n  relation = \"peer\"\n}",
            "must be a list of references",
        ),
        (
            "field a {\n  kind = \"reference\"\n  target = \"other\"\n  relation = \"peer\"\n  multiple = true\n}",
            "must target its own type",
        ),
        (
            "field a {\n  kind = \"reference\"\n  target = \"*\"\n  relation = \"two_way\"\n  inverse = \"b\"\n}",
            "only with relation \"one_way\"",
        ),
        (
            "field a {\n  kind = \"reference\"\n  target = \"t\"\n  inverse = \"b\"\n}",
            "used only by a two_way",
        ),
        (
            "field a {\n  kind = \"reference\"\n  target = \"t\"\n  on_delete = \"orphan\"\n}",
            "is not one of",
        ),
        (
            "field a {\n  kind = \"string\"\n}\nfield a {\n  kind = \"int\"\n}",
            "defined twice",
        ),
        (
            "handler \"on_save\" {\n  script = \"s\"\n}",
            "must be one event",
        ),
        (
            "handler \"before_create\" {\n  script = \"s\"\n  secrets = [\"k\"]\n}",
            "only on after_* handlers",
        ),
    ];
    for (field, expected) in cases {
        let issues = def(field);
        assert!(
            issues.iter().any(|i| i.contains(expected)),
            "`{field}` should fail with {expected:?}, got {issues:?}"
        );
    }
}

#[test]
fn cross_file_rules() {
    let tpl = "template default {\n  tab \"T\" {\n    slot \"title\"\n  }\n}\n";
    let err = |types: &[(&str, &str)], taxes: &[(&str, &str)]| {
        Schema::from_sources(types, taxes).unwrap_err().to_string()
    };

    let missing_target = format!(
        "label = \"A\"\nfield b {{\n  kind = \"reference\"\n  target = \"nope\"\n}}\n{tpl}"
    );
    assert!(err(&[("a", &missing_target)], &[]).contains("there is no type \"nope\""));

    let missing_tax = format!(
        "label = \"A\"\nfield s {{\n  kind = \"taxonomy\"\n  taxonomy = \"nope\"\n}}\n{tpl}"
    );
    assert!(err(&[("a", &missing_tax)], &[]).contains("there is no taxonomy \"nope\""));

    let bad_default = format!(
        "label = \"A\"\nfield s {{\n  kind = \"taxonomy\"\n  taxonomy = \"status\"\n  default = \"Nope\"\n}}\n{tpl}"
    );
    assert!(err(&[("a", &bad_default)], &[("status", STATUS)]).contains("\"Nope\" is not a term"));

    // a.owner's inverse names b.items, but b.items points back at a different field.
    let a = format!(
        "label = \"A\"\nfield owner {{\n  kind = \"reference\"\n  target = \"b\"\n  relation = \"two_way\"\n  inverse = \"items\"\n}}\nfield other {{\n  kind = \"string\"\n}}\n{tpl}"
    );
    let b = format!(
        "label = \"B\"\nfield items {{\n  kind = \"reference\"\n  target = \"a\"\n  relation = \"two_way\"\n  inverse = \"other\"\n  multiple = true\n}}\n{tpl}"
    );
    let e = err(&[("a", &a), ("b", &b)], &[]);
    assert!(
        e.contains("b.items must be a two_way reference to a whose inverse is \"owner\""),
        "{e}"
    );

    // The matching pair passes.
    let b_ok = b.replace("inverse = \"other\"", "inverse = \"owner\"");
    Schema::from_sources(&[("a", &a), ("b", &b_ok)], &[]).unwrap();
}

#[test]
fn note_level_rules() {
    let notes = lookup(&[(BOX_NOTE, "box")]);
    let schema = kinds();
    let check = |src: &str| schema.validate_note_source("kinds", src, "n.wcl", &notes);

    let no_title = check("s = \"x\"\n").unwrap_err();
    assert!(
        no_title.to_string().contains("title: is required"),
        "{no_title}"
    );
    let empty_title = check("title = \"  \"\n").unwrap_err();
    assert!(
        empty_title.to_string().contains("cannot be empty"),
        "{empty_title}"
    );
    let bad_created =
        check("title = \"t\"\ncreated = \"2026-10-12T09:15:00+11:00\"\n").unwrap_err();
    assert!(
        bad_created.to_string().contains("UTC datetime"),
        "{bad_created}"
    );
    let compound_as_value = check("title = \"t\"\nbody = \"text\"\n").unwrap_err();
    assert!(
        compound_as_value
            .to_string()
            .contains("written as a `content body` block"),
        "{compound_as_value}"
    );
    let wrong_block = check("title = \"t\"\ncanvas sketch {\n  viewport {\n    x = 0.0\n    y = 0.0\n    zoom = 1.0\n  }\n}\n").unwrap_err();
    assert!(
        wrong_block.to_string().contains("is a ink field"),
        "{wrong_block}"
    );
    let bad_archived = check("title = \"t\"\narchived = \"yes\"\n").unwrap_err();
    assert!(
        bad_archived
            .to_string()
            .contains("archived: must be true or false"),
        "{bad_archived}"
    );
    let archived = check("title = \"t\"\narchived = true\n").unwrap();
    assert!(!archived.is_flagged(), "archived is built in, not unknown");
    let not_data = check("title = \"t\"\ni = 1 + 2\n").unwrap_err();
    assert!(not_data.to_string().contains("expression"), "{not_data}");

    // Data the type no longer describes is kept and flagged, not rejected (D6).
    let report = check("title = \"t\"\nold_field = 3\ndict old_dict {\n  a = 1\n}\n").unwrap();
    assert!(report.is_flagged());
    assert_eq!(report.unknown, ["old_field", "dict \"old_dict\""]);
}

#[test]
fn required_fields_must_be_present() {
    let ty = "label = \"R\"\nfield n {\n  kind = \"int\"\n  required = true\n  default = 1\n}\nfield tags {\n  kind = \"select\"\n  options = [\"a\"]\n  multiple = true\n  required = true\n}\ntemplate default {\n  tab \"T\" {\n    slot \"n\"\n  }\n}\n";
    let schema = Schema::from_sources(&[("r", ty)], &[]).unwrap();
    let none = lookup(&[]);
    let err = schema
        .validate_note_source("r", "title = \"t\"\ntags = []\n", "n", &none)
        .unwrap_err()
        .to_string();
    assert!(err.contains("n: is required"), "{err}");
    assert!(
        err.contains("tags: is required and cannot be empty"),
        "{err}"
    );
    schema
        .validate_note_source("r", "title = \"t\"\nn = 2\ntags = [\"a\"]\n", "n", &none)
        .unwrap();
}

#[test]
fn an_unknown_type_fails() {
    let err = example()
        .validate_note_source("nope", "title = \"t\"\n", "n", &lookup(&[]))
        .unwrap_err();
    assert!(err.to_string().contains("there is no type \"nope\""));
}

/// An asset lookup that knows of no assets.
struct NoAssets;

impl NoteLookup for NoAssets {
    fn note_type(&self, _id: &str) -> Option<String> {
        None
    }

    fn asset_exists(&self, _id: &str) -> bool {
        false
    }
}

#[test]
fn an_asset_must_exist() {
    let err = kinds()
        .validate_note_source(
            "kinds",
            &format!("title = \"t\"\na = {ASSET:?}\n"),
            "n",
            &NoAssets,
        )
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[test]
fn load_reads_a_vault() {
    let dir = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("types/specimen.wcl"), SPECIMEN_TYPE).unwrap();
    std::fs::write(dir.path().join("types/board.wcl"), BOARD_TYPE).unwrap();
    std::fs::write(dir.path().join("taxonomies/status.wcl"), STATUS).unwrap();
    std::fs::write(dir.path().join("taxonomies/topic.wcl"), TOPIC).unwrap();
    let schema = Schema::load(&vault).unwrap();
    assert_eq!(schema.types().count(), 2);
    assert_eq!(schema.taxonomies().count(), 2);

    std::fs::remove_file(dir.path().join("taxonomies/topic.wcl")).unwrap();
    let err = Schema::load(&vault).unwrap_err().to_string();
    assert!(
        err.contains("types/specimen.wcl") && err.contains("\"topic\""),
        "{err}"
    );
}

#[test]
fn a_type_has_only_default_and_card_templates_and_both_can_have_tabs() {
    let tabs = "label = \"T\"\nfield a {\n  kind = \"string\"\n}\ntemplate default {\n  tab \"One\" {\n    slot \"title\"\n  }\n  tab \"Two\" {\n    slot \"a\"\n  }\n}\ntemplate card {\n  tab \"Front\" {\n    slot \"title\"\n  }\n  tab \"Back\" {\n    slot \"a\"\n  }\n}\n";
    let ty = TypeDef::parse("t", tabs).unwrap();
    assert_eq!(ty.templates.len(), 2);
    assert!(ty.templates.iter().all(|t| t.tabs.len() == 2));

    for other in ["detail", "wide"] {
        let issues = type_issues(&format!(
            "label = \"T\"\ntemplate default {{\n  tab \"Main\" {{\n    slot \"title\"\n  }}\n}}\ntemplate {other} {{\n  tab \"Main\" {{\n    slot \"title\"\n  }}\n}}\n"
        ));
        assert!(
            issues
                .iter()
                .any(|i| i.contains("only `default` and `card`")),
            "{other}: {issues:?}"
        );
    }
}

/// A canvas field `plan` holding `items` after its viewport.
fn plan(items: &str) -> String {
    format!(
        "canvas plan {{\n  viewport {{\n    x = 0.0\n    y = 0.0\n    zoom = 1.0\n  }}\n{items}\n}}"
    )
}

const CARD: &str = "  node c {\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 200.0\n    height = 100.0\n    colour = \"green\"\n    heading {\n      level = 2\n      text = \"Plan\"\n    }\n    paragraph {\n      text = \"Some **bold**\"\n    }\n    code {\n      language = \"rust\"\n      source = \"fn main() {}\"\n    }\n    image {\n      asset = \"01JB9A0C3D4E5F6G7H8J9K0M1N\"\n    }\n    query_view {\n      query = \"from box\"\n      view cards {\n        kind = \"cards\"\n      }\n    }\n  }\n";

#[test]
fn a_card_holds_content_blocks_and_is_checked_like_a_content_field() {
    check_kinds(&plan(CARD)).unwrap();
    // An empty card is fine.
    check_kinds(&plan(
        "  node c {\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n  }\n",
    ))
    .unwrap();

    // Its blocks are checked as content blocks are.
    let bad = check_kinds(&plan(
        "  node c {\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    heading {\n      level = 9\n      text = \"x\"\n    }\n    stroke {\n      tool = \"pen\"\n    }\n  }\n",
    ))
    .unwrap_err();
    assert!(
        bad.iter()
            .any(|i| i.contains("heading#1.level: must be from 1 to 3")),
        "{bad:?}"
    );
    assert!(
        bad.iter()
            .any(|i| i.contains("\"stroke\" is not a content block kind")),
        "{bad:?}"
    );

    // A card cannot hold a canvas, and a note node holds no blocks.
    let nested = check_kinds(&plan(&format!(
        "  node c {{\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n{}\n  }}\n",
        "    canvas {\n      viewport {\n        x = 0.0\n        y = 0.0\n        zoom = 1.0\n      }\n    }"
    )))
    .unwrap_err();
    assert!(
        nested
            .iter()
            .any(|i| i.contains("a card cannot hold a canvas")),
        "{nested:?}"
    );
    let note_blocks = check_kinds(&plan(&format!(
        "  node n {{\n    kind = \"note\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    note = \"{BOX_NOTE}\"\n    paragraph {{\n      text = \"x\"\n    }}\n  }}\n"
    )))
    .unwrap_err();
    assert!(
        note_blocks
            .iter()
            .any(|i| i.contains("is not allowed here")),
        "{note_blocks:?}"
    );

    // The kinds cards replaced are not node kinds any more, and their items are not node items.
    let old = check_kinds(&plan(
        "  node c {\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    text = \"hi\"\n  }\n  node g {\n    kind = \"group\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n  }\n",
    ))
    .unwrap_err();
    assert!(
        old.iter()
            .any(|i| i.contains("c\".text: is not an item of this block")),
        "{old:?}"
    );
    assert!(
        old.iter()
            .any(|i| i.contains("\"group\" is not one of card, note")),
        "{old:?}"
    );
}

#[test]
fn edges_take_four_arrow_values_and_canvases_a_height() {
    let edge = |arrow: &str| {
        plan(&format!(
            "{CARD}  edge e {{\n    from = \"c\"\n    to = \"c\"\n    arrow = \"{arrow}\"\n    label = \"caption\"\n    colour = \"red\"\n  }}\n"
        ))
    };
    for arrow in ["none", "start", "end", "both"] {
        check_kinds(&edge(arrow)).unwrap_or_else(|e| panic!("{arrow}: {e:?}"));
    }
    let bad = check_kinds(&edge("middle")).unwrap_err();
    assert!(
        bad.iter()
            .any(|i| i.contains("\"middle\" is not one of none, start, end, both")),
        "{bad:?}"
    );

    let height = |h: &str| {
        format!(
            "canvas plan {{\n  height = {h}\n  viewport {{\n    x = 0.0\n    y = 0.0\n    zoom = 1.0\n  }}\n}}"
        )
    };
    check_kinds(&height("480")).unwrap();
    check_kinds(&height("160")).unwrap();
    check_kinds(&height("4000")).unwrap();
    for bad in ["159", "4001", "-1"] {
        let issues = check_kinds(&height(bad)).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("plan\".height: must be from 160 to 4000")),
            "{bad}: {issues:?}"
        );
    }
    let float = check_kinds(&height("300.5")).unwrap_err();
    assert!(
        float.iter().any(|i| i.contains("must be an integer")),
        "{float:?}"
    );
}

#[test]
fn an_ink_drawing_takes_an_optional_height_up_to_4000() {
    let field = |items: &str| {
        format!(
            "ink sketch {{\n  width = 800\n{items}  stroke {{\n    tool = \"pen\"\n    colour = \"#1f2937\"\n    size = 2.0\n    points = [1.0, 2.0, 0.5, 10.0, 3900.0, 0.5]\n  }}\n}}"
        )
    };
    let block =
        |items: &str| format!("content body {{\n  ink {{\n    width = 800\n{items}  }}\n}}");
    // Omitted reads as the default; a drawing made short before the handle existed stays valid.
    check_kinds(&field("")).unwrap();
    check_kinds(&block("")).unwrap();
    for ok in ["1", "80", "400", "4000"] {
        check_kinds(&field(&format!("  height = {ok}\n")))
            .unwrap_or_else(|e| panic!("{ok}: {e:?}"));
        check_kinds(&block(&format!("    height = {ok}\n")))
            .unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in ["0", "4001", "-5"] {
        let issues = check_kinds(&field(&format!("  height = {bad}\n"))).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("sketch\".height: must be from 1 to 4000")),
            "{bad}: {issues:?}"
        );
        let issues = check_kinds(&block(&format!("    height = {bad}\n"))).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("height: must be from 1 to 4000")),
            "{bad}: {issues:?}"
        );
    }
    let float = check_kinds(&field("  height = 300.5\n")).unwrap_err();
    assert!(
        float.iter().any(|i| i.contains("must be an integer")),
        "{float:?}"
    );
    assert_eq!(
        (
            schema::INK_HEIGHT_DEFAULT,
            schema::INK_HEIGHT_MIN,
            schema::INK_HEIGHT_MAX
        ),
        (400, 1, 4000)
    );
}

#[test]
fn an_ink_drawing_takes_an_optional_viewport_and_strokes_anywhere_on_its_plane() {
    let stroke = "    stroke {\n      tool = \"pen\"\n      colour = \"#1f2937\"\n      size = 4.0\n      points = [-1200.5, -40.25, 0.5, 98000.0, 12.0, 0.6]\n    }\n";
    let block = |items: &str| format!("content body {{\n  ink {{\n{items}{stroke}  }}\n}}");
    // No width and no viewport: an endless drawing at its default view.
    check_kinds(&block("")).unwrap();
    check_kinds(&block("    width = 800\n    height = 300\n")).unwrap();
    check_kinds(&block(
        "    viewport {\n      x = -1500.0\n      y = -80.5\n      zoom = 0.25\n    }\n",
    ))
    .unwrap();
    check_kinds(&format!(
        "ink sketch {{\n  viewport {{\n    x = 10.0\n    y = 20.0\n    zoom = 8.0\n  }}\n{stroke}}}"
    ))
    .unwrap();

    let bad = |items: &str, want: &str| {
        let issues = check_kinds(&block(items)).unwrap_err();
        assert!(
            issues.iter().any(|i| i.contains(want)),
            "{items}: {issues:?}"
        );
    };
    let vp = |body: &str| format!("    viewport {{\n{body}    }}\n");
    bad(
        &vp("      x = 0.0\n      y = 0.0\n      zoom = 0.0\n"),
        "zoom",
    );
    bad(
        &vp("      x = 0.0\n      zoom = 1.0\n"),
        "viewport.y: is required",
    );
    bad(
        &vp("      x = 0.0\n      y = 0.0\n      zoom = 1.0\n      scale = 2.0\n"),
        "scale",
    );
    let one = vp("      x = 0.0\n      y = 0.0\n      zoom = 1.0\n");
    bad(&format!("{one}{one}"), "has more than one viewport");
}

#[test]
fn a_stroke_colour_is_a_hex_or_the_default_ink_token() {
    let block = |colour: &str| {
        format!(
            "content body {{\n  ink {{\n    stroke {{\n      tool = \"pen\"\n      colour = \"{colour}\"\n      size = 4.0\n      points = [1.0, 2.0, 0.5]\n    }}\n  }}\n}}"
        )
    };
    check_kinds(&block("ink")).unwrap();
    check_kinds(&block("#1f2937")).unwrap();
    for bad in ["Ink", "black", "#1F2937", "inky"] {
        let issues = check_kinds(&block(bad)).unwrap_err();
        assert!(
            issues.iter().any(|i| i.contains("colour")),
            "{bad}: {issues:?}"
        );
    }
}

#[test]
fn an_asset_block_takes_a_viewer_height_and_a_card_flag() {
    let block = |items: &str| {
        format!(
            "content body {{\n  asset {{\n    asset = \"01JB9A0C3D4E5F6G7H8J9K0M1N\"\n{items}  }}\n}}"
        )
    };
    check_kinds(&block("")).unwrap();
    check_kinds(&block("    height = 640\n    card = true\n")).unwrap();
    let issues = check_kinds(&block("    height = 100\n")).unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.contains("must be from 160 to 4000")),
        "{issues:?}"
    );
    let issues = check_kinds(&block("    card = \"yes\"\n")).unwrap_err();
    assert!(
        issues.iter().any(|i| i.contains("must be true or false")),
        "{issues:?}"
    );
}

#[test]
fn a_card_block_names_a_note_by_ulid_which_need_not_exist() {
    let block = |items: &str| format!("content body {{\n  card {{\n{items}  }}\n}}");
    check_kinds(&block(&format!("    note = {BOX_NOTE:?}\n"))).unwrap();
    // A card of a note that is gone stays valid, as a mention of one does.
    check_kinds(&block("    note = \"01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ\"\n")).unwrap();
    // Nested in a toggle, as anywhere else in the content.
    check_kinds(&format!(
        "content body {{\n  toggle {{\n    text = \"t\"\n\n    card {{\n      note = {BOX_NOTE:?}\n    }}\n  }}\n}}"
    ))
    .unwrap();
    let issues = check_kinds(&block("")).unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.ends_with("card#1.note: is required")),
        "{issues:?}"
    );
    let issues = check_kinds(&block("    note = \"nope\"\n")).unwrap_err();
    assert!(
        issues.iter().any(|i| i.contains("\"nope\" is not a ULID")),
        "{issues:?}"
    );
    let issues = check_kinds(&block(&format!(
        "    note = {BOX_NOTE:?}\n    template = \"card\"\n"
    )))
    .unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.contains("card#1.template: is not an item of this block")),
        "{issues:?}"
    );
}

#[test]
fn a_map_view_takes_a_height_from_160_to_4000_and_no_other_view_does() {
    let block = |kind: &str, items: &str| {
        format!(
            "content body {{\n  query_view {{\n    query = \"from kinds\"\n    view v {{\n      kind = \"{kind}\"\n{items}    }}\n  }}\n}}"
        )
    };
    let map = |items: &str| block("map", &format!("      location = \"geo\"\n{items}"));
    for ok in [
        "",
        "      height = 160\n",
        "      height = 900\n",
        "      height = 4000\n",
    ] {
        check_kinds(&map(ok)).unwrap();
    }
    for bad in ["159", "4001"] {
        let issues = check_kinds(&map(&format!("      height = {bad}\n"))).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("view \"v\".height: must be from 160 to 4000")),
            "{bad}: {issues:?}"
        );
    }
    let issues = check_kinds(&map("      height = 600.5\n")).unwrap_err();
    assert!(
        issues.iter().any(|i| i.contains("must be an integer")),
        "{issues:?}"
    );
    for (kind, items) in [
        ("table", ""),
        ("cards", ""),
        ("kanban", "      group = \"tax\"\n"),
        ("calendar", "      date = \"d\"\n"),
        ("canvas", ""),
    ] {
        let issues =
            check_kinds(&block(kind, &format!("{items}      height = 600\n"))).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|i| i.contains(&format!("view \"v\".height: is not used by a {kind} view"))),
            "{kind}: {issues:?}"
        );
    }
}

#[test]
fn a_canvas_in_the_old_format_reads_as_cards_and_validates() {
    let old = plan(&format!(
        "  node t {{\n    kind = \"text\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    colour = \"blue\"\n    text = \"Hello **you**\"\n  }}\n  node a {{\n    kind = \"asset\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    asset = \"{ASSET}\"\n  }}\n  node l {{\n    kind = \"link\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    url = \"https://example.com\"\n  }}\n  node q {{\n    kind = \"query\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n    query = \"from box\"\n    view = \"table\"\n  }}\n  node i {{\n    kind = \"ink\"\n    x = 0.0\n    y = 0.0\n    width = 10.0\n    height = 10.0\n  }}\n  edge e {{\n    from = \"t\"\n    to = \"q\"\n    arrow = \"both\"\n  }}\n"
    ));
    check_kinds(&old).unwrap();
    let doc = schema::Doc::parse(&format!("title = \"t\"\n{old}"), "n.wcl").unwrap();
    let canvas = &doc.blocks[0].body;
    let nodes: Vec<(String, Vec<&str>)> = canvas
        .blocks_of("node")
        .map(|n| {
            (
                n.body
                    .field("kind")
                    .and_then(schema::Value::as_str)
                    .unwrap()
                    .to_owned(),
                n.body.blocks.iter().map(|b| b.kind.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        nodes,
        [
            ("card".to_owned(), vec!["paragraph"]),
            ("card".to_owned(), vec!["image"]),
            ("card".to_owned(), vec!["paragraph"]),
            ("card".to_owned(), vec!["query_view"]),
            ("card".to_owned(), vec!["ink"]),
        ]
    );
    let t = canvas.blocks_of("node").next().unwrap();
    assert_eq!(t.label(), Some("t"));
    assert_eq!(
        t.body.field("colour").and_then(schema::Value::as_str),
        Some("blue")
    );
    assert_eq!(canvas.blocks_of("edge").count(), 1);
}

const TAG_TYPE: &str = "label = \"Post\"\nfield tags {\n  kind     = \"list\"\n  items    = \"taxonomy\"\n  taxonomy = \"status\"\n}\nfield topics {\n  kind     = \"taxonomy\"\n  taxonomy = \"status\"\n  multiple = true\n}\ntemplate default {\n  tab \"T\" {\n    slot \"tags\"\n  }\n}\n";

#[test]
fn a_list_of_terms_reads_as_a_multiple_taxonomy_field() {
    let schema = Schema::from_sources(&[("post", TAG_TYPE)], &[("status", STATUS)]).unwrap();
    let post = schema.type_def("post").unwrap();
    let tags = post.field("tags").unwrap();
    let topics = post.field("topics").unwrap();
    assert_eq!(
        tags.kind,
        FieldKind::Taxonomy {
            taxonomy: "status".into()
        }
    );
    assert!(tags.multiple && tags.listed);
    // Apart from how it was declared, it is the same field as a multiple taxonomy field.
    assert_eq!(tags.kind, topics.kind);
    assert!(topics.multiple && !topics.listed);
    assert!(!tags.is_kanban_group());

    // Its values are term paths, validated as a multiple taxonomy field's are.
    let none = lookup(&[]);
    let check = |body: &str| {
        schema
            .validate_note_source("post", &format!("title = \"t\"\n{body}\n"), "n", &none)
            .map(|_| ())
            .map_err(|e| text(&e.issues))
    };
    check(r#"tags = ["Inbox", "Active/In progress"]"#).unwrap();
    for (body, says) in [
        (
            r#"tags = ["Inbox", "Nope"]"#,
            "tags[1]: \"Nope\" is not a term of taxonomy \"status\"",
        ),
        (
            r#"tags = ["Inbox", "Inbox"]"#,
            "tags: lists \"Inbox\" twice",
        ),
        (
            r#"tags = "Inbox""#,
            "tags: holds several values and must be a list",
        ),
        (r#"tags = [1]"#, "tags[0]: must be a term path string"),
    ] {
        let issues = check(body).unwrap_err();
        assert!(
            issues.iter().any(|i| i.starts_with(says)),
            "{body}: {issues:?}"
        );
    }
}

#[test]
fn a_list_of_terms_names_its_taxonomy_and_takes_no_multiple() {
    let tpl = "template default {\n  tab \"T\" {\n    slot \"title\"\n  }\n}\n";
    let field = |body: &str| format!("label = \"A\"\nfield f {{\n{body}\n}}\n{tpl}");

    let issues = type_issues(&field("  kind = \"list\"\n  items = \"taxonomy\""));
    assert!(
        issues.iter().any(|i| i == "field f.taxonomy: is required"),
        "{issues:?}"
    );
    // `taxonomy` is not an attribute of a list of strings: a warning, and the field still loads.
    let t = TypeDef::parse(
        "a",
        &field("  kind = \"list\"\n  items = \"string\"\n  taxonomy = \"status\""),
    )
    .unwrap();
    assert_eq!(
        text(&t.warnings),
        [
            "warning: field f.taxonomy: is not an attribute of list fields in this version; it is ignored"
        ]
    );
    let issues = type_issues(&field(
        "  kind = \"list\"\n  items = \"taxonomy\"\n  taxonomy = \"status\"\n  multiple = true",
    ));
    assert!(
        issues
            .iter()
            .any(|i| i.contains("multiple: is not allowed on a list field")),
        "{issues:?}"
    );
    let issues = type_issues(&field("  kind = \"list\"\n  items = \"term\""));
    assert!(
        issues.iter().any(|i| i.starts_with("field f.items")),
        "{issues:?}"
    );

    // The taxonomy must exist, and a default must hold its terms.
    let missing = field("  kind = \"list\"\n  items = \"taxonomy\"\n  taxonomy = \"nope\"");
    let e = Schema::from_sources(&[("a", &missing)], &[])
        .unwrap_err()
        .to_string();
    assert!(e.contains("there is no taxonomy \"nope\""), "{e}");
    let bad_default = field(
        "  kind = \"list\"\n  items = \"taxonomy\"\n  taxonomy = \"status\"\n  default = [\"Inbox\", \"Nope\"]",
    );
    let e = Schema::from_sources(&[("a", &bad_default)], &[("status", STATUS)])
        .unwrap_err()
        .to_string();
    assert!(e.contains("\"Nope\" is not a term"), "{e}");
    let good_default = bad_default.replace("\"Nope\"", "\"Done\"");
    Schema::from_sources(&[("a", &good_default)], &[("status", STATUS)]).unwrap();
}

#[test]
fn an_open_taxonomy_may_start_empty_and_a_closed_one_may_not() {
    use schema::Taxonomy;
    let open = Taxonomy::parse("tags", "label = \"Tags\"\nallow_new = true\n").unwrap();
    assert!(open.allow_new);
    assert!(open.terms.is_empty());
    let closed = Taxonomy::parse("tags", "label = \"Tags\"\nallow_new = false\n").unwrap_err();
    assert!(
        closed.to_string().contains("needs at least one term"),
        "{closed}"
    );
    let wrong = Taxonomy::parse(
        "tags",
        "label = \"Tags\"\nallow_new = \"yes\"\nterm \"A\"\n",
    )
    .unwrap_err();
    assert!(wrong.to_string().contains("allow_new"), "{wrong}");
    assert!(!Taxonomy::parse("status", STATUS).unwrap().allow_new);
}

#[test]
fn a_placeholder_link_in_template_text_must_name_a_type() {
    let ty = |target: &str| {
        format!(
            "label = \"T\"\ntemplate default {{\n  tab \"Main\" {{\n    paragraph {{\n      text = \"Start [[+{target}|First step]]\"\n    }}\n  }}\n}}\n"
        )
    };
    Schema::from_sources(&[("t", &ty("t"))], &[]).expect("a placeholder for its own type");
    let err = Schema::from_sources(&[("t", &ty("nope"))], &[])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("template default > tab#1 > paragraph#1.text")
            && err.contains("names type \"nope\", which does not exist"),
        "{err}"
    );
}

#[test]
fn a_hidden_field_is_read_and_defaults_to_shown() {
    let schema = Schema::from_sources(
        &[(
            "person",
            "label = \"Person\"\n\nfield phones {\n  kind  = \"list\"\n  items = \"string\"\n}\n\nfield device_contacts {\n  kind   = \"dict\"\n  values = \"string\"\n  hidden = true\n}\n\ntemplate default {\n  tab \"P\" {\n    slot \"title\"\n  }\n}\n",
        )],
        &[],
    )
    .unwrap();
    let person = schema.type_def("person").unwrap();
    assert!(person.field("device_contacts").unwrap().hidden);
    assert!(!person.field("phones").unwrap().hidden);
}

/// A type with the given fields and a default template placing the title.
fn type_with(fields: &str) -> String {
    format!(
        "label = \"T\"\n{fields}\ntemplate default {{\n  tab \"T\" {{\n    slot \"title\"\n  }}\n}}\n"
    )
}

const STRINGS: &str = "  kind  = \"list\"\n  items = \"string\"";

#[test]
fn labels_names_another_list_of_strings() {
    let source = type_with(&format!(
        "field phones {{\n{STRINGS}\n  labels = \"phone_labels\"\n}}\nfield phone_labels {{\n{STRINGS}\n}}\n"
    ));
    let t = TypeDef::parse("t", &source).unwrap();
    assert_eq!(
        t.field("phones").unwrap().labels.as_deref(),
        Some("phone_labels")
    );
    assert_eq!(t.field("phone_labels").unwrap().labels, None);
}

#[test]
fn labels_is_refused_when_it_names_a_missing_field_a_non_list_itself_or_a_named_field() {
    let cases = [
        (
            format!("field a {{\n{STRINGS}\n  labels = \"nope\"\n}}\n"),
            "field a.labels: the type has no field \"nope\"",
        ),
        (
            format!(
                "field a {{\n{STRINGS}\n  labels = \"b\"\n}}\nfield b {{\n  kind = \"string\"\n}}\n"
            ),
            "field a.labels: \"b\" must be a list of strings, and it is a string field",
        ),
        (
            format!(
                "field a {{\n{STRINGS}\n  labels = \"b\"\n}}\nfield b {{\n  kind  = \"list\"\n  items = \"int\"\n}}\n"
            ),
            "field a.labels: \"b\" must be a list of strings",
        ),
        (
            format!("field a {{\n{STRINGS}\n  labels = \"a\"\n}}\n"),
            "field a.labels: cannot name the field itself",
        ),
        (
            format!(
                "field a {{\n{STRINGS}\n  labels = \"l\"\n}}\nfield b {{\n{STRINGS}\n  labels = \"l\"\n}}\nfield l {{\n{STRINGS}\n}}\n"
            ),
            "field b.labels: \"l\" already holds the labels of field \"a\"",
        ),
    ];
    for (fields, want) in cases {
        let issues = type_issues(&type_with(&fields));
        assert!(
            issues.iter().any(|i| i.contains(want)),
            "{want}: {issues:?}"
        );
    }
    // `labels` belongs to a list of strings only: on a list of ints it is ignored, with a warning.
    let t = TypeDef::parse(
        "t",
        &type_with(
            "field a {\n  kind  = \"list\"\n  items = \"int\"\n  labels = \"l\"\n}\nfield l {\n  kind  = \"list\"\n  items = \"string\"\n}\n",
        ),
    )
    .unwrap();
    assert_eq!(
        text(&t.warnings),
        [
            "warning: field a.labels: is not an attribute of list fields in this version; it is ignored"
        ]
    );
    assert_eq!(t.field("a").unwrap().labels, None);
}

/// Assets by id with their file names.
struct Files(HashMap<String, String>);

impl NoteLookup for Files {
    fn note_type(&self, _id: &str) -> Option<String> {
        None
    }

    fn asset_exists(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }

    fn asset_filename(&self, id: &str) -> Option<String> {
        self.0.get(id).cloned()
    }
}

#[test]
fn an_image_asset_field_refuses_an_asset_that_is_not_an_image() {
    const PDF: &str = "01JB9A0C3D4E5F6G7H8J9K0M1P";
    let source = type_with(
        "field photo {\n  kind   = \"asset\"\n  accept = \"image\"\n}\nfield file {\n  kind = \"asset\"\n}\n",
    );
    let schema = Schema::from_sources(&[("t", &source)], &[]).unwrap();
    assert_eq!(
        schema.type_def("t").unwrap().field("photo").unwrap().accept,
        schema::Accept::Image
    );
    assert_eq!(
        schema.type_def("t").unwrap().field("file").unwrap().accept,
        schema::Accept::Any
    );
    let files = Files(HashMap::from([
        (ASSET.to_owned(), "Face.JPG".to_owned()),
        (PDF.to_owned(), "cv.pdf".to_owned()),
    ]));
    let note = |field: &str, id: &str| format!("title = \"t\"\n{field} = {id:?}\n");
    schema
        .validate_note_source("t", &note("photo", ASSET), "n", &files)
        .unwrap();
    schema
        .validate_note_source("t", &note("file", PDF), "n", &files)
        .unwrap();
    let err = schema
        .validate_note_source("t", &note("photo", PDF), "n", &files)
        .unwrap_err();
    assert!(err.to_string().contains("holds only images"), "{err}");

    let issues = type_issues(&type_with(
        "field photo {\n  kind   = \"asset\"\n  accept = \"video\"\n}\n",
    ));
    assert!(
        issues.iter().any(|i| i.starts_with("field photo.accept")),
        "{issues:?}"
    );
}

/// A type with one field `f` whose body is `field`, `top` at the top of the file.
fn tolerant_type(top: &str, field: &str) -> String {
    format!(
        "label = \"A\"\n{top}\nfield f {{\n{field}\n}}\ntemplate default {{\n  tab \"T\" {{\n    slot \"title\"\n    slot \"f\"\n  }}\n}}\n"
    )
}

#[test]
fn an_unknown_field_attribute_is_a_warning_and_the_type_still_loads_and_validates() {
    let source = tolerant_type(
        "",
        "  kind = \"int\"\n  required = true\n  sparkle = \"gold\"",
    );
    let t = TypeDef::parse("a", &source).unwrap();
    assert_eq!(
        text(&t.warnings),
        [
            "warning: field f.sparkle: is not an attribute of int fields in this version; it is ignored"
        ]
    );
    assert!(t.warnings.iter().all(Issue::is_warning));
    let f = t.field("f").expect("the field still loads");
    assert_eq!(f.kind, FieldKind::Int);
    assert!(f.required, "the attributes it knows still apply");

    let schema = Schema::from_sources(&[("a", &source)], &[]).unwrap();
    let none = lookup(&[]);
    schema
        .validate_note_source("a", "title = \"t\"\nf = 3\n", "n.wcl", &none)
        .unwrap();
    let err = schema
        .validate_note_source("a", "title = \"t\"\nf = \"three\"\n", "n.wcl", &none)
        .unwrap_err();
    assert!(err.to_string().contains("f: must be"), "{err}");
    let err = schema
        .validate_note_source("a", "title = \"t\"\n", "n.wcl", &none)
        .unwrap_err();
    assert!(err.to_string().contains("is required"), "{err}");
}

#[test]
fn an_unknown_type_attribute_is_a_warning_and_the_type_still_loads_and_validates() {
    let source = tolerant_type("sort_by = \"title\"\nlayout = 2", "  kind = \"string\"");
    let t = TypeDef::parse("a", &source).unwrap();
    assert_eq!(
        text(&t.warnings),
        [
            "warning: type.sort_by: is not an attribute of types in this version; it is ignored",
            "warning: type.layout: is not an attribute of types in this version; it is ignored",
        ]
    );
    assert_eq!(t.label, "A");
    let schema = Schema::from_sources(&[("a", &source)], &[]).unwrap();
    let none = lookup(&[]);
    schema
        .validate_note_source("a", "title = \"t\"\nf = \"x\"\n", "n.wcl", &none)
        .unwrap();
    let err = schema
        .validate_note_source("a", "title = \"t\"\nf = 3\n", "n.wcl", &none)
        .unwrap_err();
    assert!(err.to_string().contains("f: must be"), "{err}");
}

#[test]
fn a_malformed_value_of_a_known_attribute_stays_an_error_beside_an_unknown_one() {
    for (field, says) in [
        (
            "  kind = \"asset\"\n  accept = \"video\"",
            "field f.accept: \"video\" is not one of",
        ),
        (
            "  kind = \"int\"\n  required = \"yes\"",
            "field f.required: must be true or false",
        ),
        (
            "  kind = \"list\"\n  items = \"string\"\n  labels = 3",
            "field f.labels: must be a string",
        ),
        (
            "  kind = \"glitter\"",
            "field f.kind: \"glitter\" is not one of",
        ),
    ] {
        let source = tolerant_type("", &format!("{field}\n  sparkle = true"));
        let issues = type_issues(&source);
        assert!(
            issues.iter().any(|i| i.starts_with(says)),
            "{field}: {issues:?}"
        );
        assert!(
            issues.iter().all(|i| !i.starts_with("warning:")),
            "a refused type reports its errors: {issues:?}"
        );
    }
    let issues = type_issues(&tolerant_type(
        "colour = \"teal\"\nsparkle = 1",
        "  kind = \"string\"",
    ));
    assert!(
        issues
            .iter()
            .any(|i| i.starts_with("type.colour: \"teal\" is not a palette colour")),
        "{issues:?}"
    );
}

#[test]
fn the_newer_person_type_loads_on_a_reader_that_lacks_its_newest_attributes() {
    // A field attribute some later release adds, on the shipped person type.
    let person = include_str!("../../writer/defaults/person.wcl").replacen(
        "  labels = \"email_labels\"\n",
        "  labels = \"email_labels\"\n  verify = \"smtp\"\n",
        1,
    );
    assert!(person.contains("verify = \"smtp\""));
    let t = TypeDef::parse("person", &person).unwrap();
    assert_eq!(
        text(&t.warnings),
        [
            "warning: field emails.verify: is not an attribute of list fields in this version; it is ignored"
        ]
    );
    assert_eq!(
        t.field("emails").unwrap().labels.as_deref(),
        Some("email_labels")
    );
}

#[test]
fn select_options_take_labels_one_per_option_in_the_same_order() {
    let source = tolerant_type(
        "",
        "  kind          = \"select\"\n  options       = [\"primary\", \"work@group.calendar.google.com\", \"plain\"]\n  option_labels = [\"Personal\", \"Work\", \"\"]",
    );
    let t = TypeDef::parse("a", &source).unwrap();
    assert!(t.warnings.is_empty(), "{:?}", text(&t.warnings));
    let f = t.field("f").unwrap();
    assert_eq!(
        f.kind,
        FieldKind::Select {
            options: vec![
                "primary".into(),
                "work@group.calendar.google.com".into(),
                "plain".into()
            ],
            option_labels: vec!["Personal".into(), "Work".into(), "".into()],
        }
    );
    assert_eq!(f.option_text("primary"), "Personal");
    assert_eq!(f.option_text("work@group.calendar.google.com"), "Work");
    assert_eq!(
        f.option_text("plain"),
        "plain",
        "an empty label shows the value"
    );
    assert_eq!(f.option_text("gone"), "gone");

    // A note still stores the value, never the label.
    let schema = Schema::from_sources(&[("a", &source)], &[]).unwrap();
    let none = lookup(&[]);
    schema
        .validate_note_source("a", "title = \"t\"\nf = \"primary\"\n", "n.wcl", &none)
        .unwrap();
    let err = schema
        .validate_note_source("a", "title = \"t\"\nf = \"Personal\"\n", "n.wcl", &none)
        .unwrap_err();
    assert!(
        err.to_string().contains("is not one of the options"),
        "{err}"
    );

    // The plain list, with no labels, is read as before.
    let plain = tolerant_type("", "  kind    = \"select\"\n  options = [\"a\", \"b\"]");
    let t = TypeDef::parse("a", &plain).unwrap();
    assert_eq!(
        t.field("f").unwrap().kind,
        FieldKind::Select {
            options: vec!["a".into(), "b".into()],
            option_labels: Vec::new(),
        }
    );
}

#[test]
fn option_labels_must_match_the_options_one_for_one() {
    let source = tolerant_type(
        "",
        "  kind          = \"select\"\n  options       = [\"a\", \"b\"]\n  option_labels = [\"A\"]",
    );
    let err = TypeDef::parse("a", &source).unwrap_err();
    assert_eq!(
        text(&err.issues),
        [
            "field f.option_labels: holds 1 labels for 2 options: give one label per option, in the same order"
        ]
    );
    let source = tolerant_type(
        "",
        "  kind          = \"select\"\n  options       = [\"a\"]\n  option_labels = [1]",
    );
    assert!(TypeDef::parse("a", &source).is_err());
}

#[test]
fn options_written_as_records_are_refused_so_labels_live_in_their_own_attribute() {
    // Readers up to 0.1.12 check `options` as a list of strings, so labels in `options` itself
    // would make them refuse the type; `option_labels` is an attribute they ignore with a warning.
    let source = tolerant_type(
        "",
        "  kind    = \"select\"\n  options = [{ value: \"primary\", label: \"Personal\" }]",
    );
    let err = TypeDef::parse("a", &source).unwrap_err();
    assert!(
        text(&err.issues)
            .iter()
            .any(|i| i.starts_with("field f.options:")),
        "{:?}",
        text(&err.issues)
    );
}

#[test]
fn unknown_attributes_are_kept_on_the_definition_as_written() {
    let source = "label = \"A\"\nsort_by = \"title\"\n\nfield f {\n  kind = \"int\"\n  sparkle = [1, 2.5, \"x\"]\n}\n\nfield g {\n  kind = \"list\"\n  items = \"int\"\n  labels = \"f\"\n}\n\ntemplate default {\n  density = \"compact\"\n  tab \"T\" {\n    pinned = true\n    slot \"title\"\n    slot \"f\" {\n      width = 2\n    }\n  }\n}\n";
    let t = TypeDef::parse("a", source).unwrap();
    assert_eq!(
        t.extra,
        [("sort_by".to_owned(), Value::Str("title".into()))]
    );
    assert_eq!(
        t.field("f").unwrap().extra,
        [(
            "sparkle".to_owned(),
            Value::List(vec![
                Value::Int(1),
                Value::Float(2.5),
                Value::Str("x".into())
            ])
        )]
    );
    // An attribute of another kind is ignored on this one, so it is kept too.
    assert_eq!(
        t.field("g").unwrap().extra,
        [("labels".to_owned(), Value::Str("f".into()))]
    );
    assert_eq!(t.field("g").unwrap().labels, None);
    let tpl = t.template("default").unwrap();
    assert_eq!(
        tpl.extra,
        [("density".to_owned(), Value::Str("compact".into()))]
    );
    let tab = &tpl.tabs[0];
    assert_eq!(tab.extra, [("pinned".to_owned(), Value::Bool(true))]);
    match &tab.items[1] {
        schema::TabItem::Slot { field, extra, .. } => {
            assert_eq!(field, "f");
            assert_eq!(extra, &[("width".to_owned(), Value::Int(2))]);
        }
        other => panic!("{other:?}"),
    }
    // Known attributes are not repeated in `extra`.
    let known =
        TypeDef::parse("a", &tolerant_type("", "  kind = \"int\"\n  label = \"F\"")).unwrap();
    assert!(known.extra.is_empty() && known.field("f").unwrap().extra.is_empty());
}

#[test]
fn the_reference_type_from_a_newer_release_loads_with_one_warning_per_unknown_attribute() {
    // docs/vault-format/check.sh passes this file with the same six warnings.
    let source = include_str!("../../../docs/vault-format/tolerated/newer-type.wcl");
    let t = TypeDef::parse("newer", source).unwrap();
    assert_eq!(t.warnings.len(), 6, "{:?}", text(&t.warnings));
    assert!(t.warnings.iter().all(Issue::is_warning));
    assert_eq!(t.extra.len(), 1);
    assert_eq!(t.field("rank").unwrap().extra.len(), 1);
    // A known attribute with the wrong form is still an error.
    let wrong = include_str!("../../../docs/vault-format/invalid/type-wrong-form.wcl");
    assert!(TypeDef::parse("wrong", wrong).is_err());
}

#[test]
fn a_two_way_reference_implies_its_missing_inverse_on_the_target_type() {
    let tpl = "template default {\n  tab \"T\" {\n    slot \"title\"\n  }\n}\n";
    let project = format!(
        "label = \"Project\"\nfield tasks {{\n  kind = \"reference\"\n  target = \"task\"\n  relation = \"two_way\"\n  inverse = \"project\"\n  multiple = true\n}}\n{tpl}"
    );
    let task = format!("label = \"Task\"\n{tpl}");
    let schema = Schema::from_sources(&[("project", &project), ("task", &task)], &[]).unwrap();
    let implied = schema.type_def("task").unwrap().field("project").unwrap();
    assert!(implied.implied);
    assert!(implied.multiple);
    let r = implied.reference().unwrap();
    assert_eq!(r.target.as_deref(), Some("project"));
    assert_eq!(r.relation, Relation::TwoWay);
    assert_eq!(r.inverse.as_deref(), Some("tasks"));
    assert!(
        !schema
            .type_def("project")
            .unwrap()
            .field("tasks")
            .unwrap()
            .implied
    );

    // Rebuilt from its own types, as a writer does, the schema implies the field once.
    let types: Vec<TypeDef> = schema.types().cloned().collect();
    let again = Schema::new(types, Vec::new()).unwrap();
    let fields = &again.type_def("task").unwrap().fields;
    assert_eq!(fields.iter().filter(|f| f.name == "project").count(), 1);

    // Without the reference, the implied field goes with it.
    let mut types: Vec<TypeDef> = again.types().cloned().collect();
    for t in &mut types {
        if t.name == "project" {
            t.fields.retain(|f| f.name != "tasks");
        }
    }
    let without = Schema::new(types, Vec::new()).unwrap();
    assert!(without.type_def("task").unwrap().field("project").is_none());

    // Written out on the target type, the field is the file's own and must match.
    let task_own = format!(
        "label = \"Task\"\nfield project {{\n  kind = \"reference\"\n  target = \"project\"\n  relation = \"two_way\"\n  inverse = \"tasks\"\n}}\n{tpl}"
    );
    let schema = Schema::from_sources(&[("project", &project), ("task", &task_own)], &[]).unwrap();
    let own = schema.type_def("task").unwrap().field("project").unwrap();
    assert!(!own.implied);
    assert!(!own.multiple);
    let task_wrong = format!("label = \"Task\"\nfield project {{\n  kind = \"string\"\n}}\n{tpl}");
    let e = Schema::from_sources(&[("project", &project), ("task", &task_wrong)], &[])
        .unwrap_err()
        .to_string();
    assert!(
        e.contains(
            "task.project must be a two_way reference to project whose inverse is \"tasks\""
        ),
        "{e}"
    );
}

const TYPED_TYPE: &str = r#"label = "Typed"

field owners {
  kind     = "list"
  items    = "reference"
  target   = "box"
}

field sizes {
  kind    = "list"
  items   = "select"
  options = ["s", "m", "l"]
}

field periods {
  kind  = "list"
  items = "date_range"
}

field matrix {
  kind = "list"

  item {
    kind  = "list"
    items = "int"
  }
}

field spec {
  kind   = "dict"
  values = "any"

  key weight {
    kind     = "float"
    required = true
  }

  key owner {
    kind   = "reference"
    target = "box"
  }

  key tags {
    kind  = "list"
    items = "string"
  }

  key extra {
    kind   = "dict"
    values = "int"
  }
}

field strict {
  kind = "dict"

  key when {
    kind = "date"
  }
}

template default {
  tab "T" {
    slot "title"
  }
}
"#;

#[test]
fn lists_hold_every_value_kind_and_dicts_declare_keys_of_any_kind() {
    let schema = Schema::from_sources(
        &[("typed", TYPED_TYPE), ("box", BOX_TYPE)],
        &[("status", STATUS)],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let t = schema.type_def("typed").unwrap();
    // A list of references or selects is that kind holding several, declared as a list.
    let owners = t.field("owners").unwrap();
    assert!(owners.multiple && owners.listed);
    assert!(owners.reference().is_some());
    assert!(t.field("sizes").unwrap().multiple);
    // Any other list keeps an item definition.
    let periods = t.field("periods").unwrap();
    assert!(matches!(
        periods.kind.item().unwrap().kind,
        FieldKind::DateRange { .. }
    ));
    let row = t.field("matrix").unwrap().kind.item().unwrap();
    assert_eq!(row.kind.item().unwrap().kind.name(), "int");
    let FieldKind::Dict { keys, values } = &t.field("spec").unwrap().kind else {
        panic!("spec is a dict")
    };
    assert_eq!(
        keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
        ["weight", "owner", "tags", "extra"]
    );
    assert_eq!(values, &schema::DictValues::Any);

    let notes = lookup(&[(BOX_NOTE, "box")]);
    let ok = format!(
        "title = \"T\"\nowners = [\"{BOX_NOTE}\"]\nsizes = [\"s\", \"l\"]\nperiods = [{{ start: \"2026-01-01\", end: \"2026-01-02\" }}]\nmatrix = [[1, 2], [3]]\n\ndict spec {{\n  weight = 2.5\n  owner  = \"{BOX_NOTE}\"\n  tags   = [\"a\", \"b\"]\n  extra  = {{ a: 1 }}\n  anything = \"goes\"\n  more  = [1, \"x\"]\n}}\n\ndict strict {{\n  when = \"2026-03-01\"\n}}\n"
    );
    schema
        .validate_note_source("typed", &ok, "typed.wcl", &notes)
        .unwrap_or_else(|e| panic!("{e:?}"));

    let check = |src: &str| {
        schema
            .validate_note_source("typed", src, "typed.wcl", &notes)
            .unwrap_err()
            .to_string()
    };
    // Each value is checked by its own definition, at any depth.
    let e = check("title = \"T\"\nsizes = [\"xl\"]\n\ndict spec {\n  weight = 1.0\n}\n");
    assert!(
        e.contains("sizes[0]") && e.contains("is not one of the options"),
        "{e}"
    );
    let e = check("title = \"T\"\nmatrix = [[1, \"two\"]]\n\ndict spec {\n  weight = 1.0\n}\n");
    assert!(e.contains("matrix[0][1]"), "{e}");
    let e =
        check("title = \"T\"\n\ndict spec {\n  weight = \"heavy\"\n  extra = { a: \"one\" }\n}\n");
    assert!(
        e.contains("dict \"spec\".weight") && e.contains("dict \"spec\".extra.a"),
        "{e}"
    );
    // A declared required key must be there, and a dict without `values` takes only its keys.
    let e = check(
        "title = \"T\"\n\ndict spec {\n  owner = \"01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ\"\n}\n\ndict strict {\n  other = 1\n}\n",
    );
    assert!(e.contains("dict \"spec\".weight: is required"), "{e}");
    assert!(
        e.contains("dict \"strict\".other: is not a key of this dict"),
        "{e}"
    );
}

#[test]
fn typed_list_and_dict_definitions_are_checked() {
    let def = |field: &str| {
        type_issues(&format!(
            "label = \"T\"\n{field}\ntemplate default {{\n  tab \"T\" {{\n    slot \"title\"\n  }}\n}}\n"
        ))
    };
    let cases: &[(&str, &str)] = &[
        ("field a {\n  kind = \"list\"\n}", "needs `items`"),
        (
            "field a {\n  kind = \"list\"\n  items = \"content\"\n}",
            "is not one of",
        ),
        (
            "field a {\n  kind = \"list\"\n  items = \"int\"\n\n  item {\n    kind = \"int\"\n  }\n}",
            "gives its item twice",
        ),
        ("field a {\n  kind = \"dict\"\n}", "needs `key` blocks"),
        (
            "field a {\n  kind = \"dict\"\n\n  key \"a-b\" {\n    kind = \"int\"\n  }\n}",
            "is not a key name",
        ),
        (
            "field a {\n  kind = \"dict\"\n\n  key b {\n    kind = \"ink\"\n  }\n}",
            "cannot be inside a list or a dict",
        ),
        (
            "field a {\n  kind = \"dict\"\n\n  key b {\n    kind = \"reference\"\n    target = \"t\"\n    relation = \"two_way\"\n    inverse = \"c\"\n  }\n}",
            "inside a dict or a nested list a reference is one_way",
        ),
        (
            "field a {\n  kind = \"list\"\n  items = \"select\"\n}",
            "options: is required",
        ),
    ];
    for (field, expected) in cases {
        let issues = def(field);
        assert!(
            issues.iter().any(|i| i.contains(expected)),
            "`{field}` should fail with {expected:?}, got {issues:?}"
        );
    }
}
