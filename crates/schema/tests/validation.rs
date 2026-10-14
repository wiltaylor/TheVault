//! Schema parsing and note validation, at the crate's public seam.

use std::collections::HashMap;

use schema::{FieldKind, Issue, NoteLookup, OnDelete, Relation, Schema, TypeDef};

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
    assert_eq!(specimen.fields.len(), 26);
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
        (
            "field Bad {\n  kind = \"string\"\n}",
            "not a valid field name",
        ),
        (
            "field a {\n  kind = \"int\"\n  required = true\n  computed = true\n}",
            "computed field cannot also be required",
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
            "field a {\n  kind = \"string\"\n  options = [\"x\"]\n}",
            "not an item of this block",
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
            "must be multiple",
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
    assert!(tags.multiple && tags.term_list);
    // Apart from how it was declared, it is the same field as a multiple taxonomy field.
    assert_eq!(tags.kind, topics.kind);
    assert!(topics.multiple && !topics.term_list);
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
    let issues = type_issues(&field(
        "  kind = \"list\"\n  items = \"string\"\n  taxonomy = \"status\"",
    ));
    assert!(
        issues
            .iter()
            .any(|i| i == "field f.taxonomy: is not an item of this block"),
        "{issues:?}"
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
