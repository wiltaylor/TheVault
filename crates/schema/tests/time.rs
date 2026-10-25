//! The `time` field kind: a 24-hour time of day with no date, as a field, a default, a list item,
//! a dict key and a dict's values.

use std::collections::HashMap;

use schema::{FieldKind, Issue, Schema, TypeDef, Value};

const TYPE: &str = r#"
label = "Shop"

field opens {
  kind    = "time"
  default = "09:00"
}

field slots {
  kind  = "list"
  items = "time"
}

field hours {
  kind = "dict"

  key open {
    kind     = "time"
    required = true
  }

  key close {
    kind = "time"
  }
}

field breaks {
  kind   = "dict"
  values = "time"
}

template default {
  tab "Shop" {
    slot "title"
  }
}
"#;

fn text(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

fn schema() -> Schema {
    Schema::from_sources(&[("shop", TYPE)], &[]).unwrap_or_else(|e| panic!("{e}"))
}

fn note(body: &str) -> Result<(), Vec<String>> {
    let source = format!("title = \"A shop\"\n{body}\n");
    schema()
        .validate_note_source("shop", &source, "shop.wcl", &HashMap::new())
        .map(|r| assert!(!r.is_flagged(), "{body}: {r:?}"))
        .map_err(|e| text(&e.issues))
}

#[test]
fn a_time_field_takes_hh_mm_and_hh_mm_ss() {
    let ty = schema();
    let opens = ty.type_def("shop").unwrap().field("opens").unwrap();
    assert_eq!(opens.kind, FieldKind::Time);
    assert_eq!(opens.kind.name(), "time");
    assert_eq!(opens.default, Some(Value::Str("09:00".into())));
    for ok in ["00:00", "09:05", "23:59", "12:30:45", "23:59:59"] {
        note(&format!("opens = {ok:?}")).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
}

#[test]
fn every_other_form_of_a_time_is_refused() {
    for bad in [
        "24:00",
        "9:05",
        "09:5",
        "09:60",
        "09:05:60",
        "09:05Z",
        "09:05:00+11:00",
        "09:05:00.5",
        "0905",
        "2026-10-20T09:05:00Z",
        "",
    ] {
        let issues = note(&format!("opens = {bad:?}")).expect_err(bad);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("opens") && i.contains("is not a 24-hour time of day")),
            "{bad}: {issues:?}"
        );
    }
    let issues = note("opens = 900").unwrap_err();
    assert!(
        issues.iter().any(|i| i.contains("must be a string")),
        "{issues:?}"
    );
}

#[test]
fn a_default_must_be_a_time() {
    let source = TYPE.replace("default = \"09:00\"", "default = \"9am\"");
    let issues = match TypeDef::parse("shop", &source) {
        Ok(t) => panic!("type parsed: {t:?}"),
        Err(e) => text(&e.issues),
    };
    assert!(
        issues
            .iter()
            .any(|i| i.contains("opens.default") && i.contains("not a 24-hour time of day")),
        "{issues:?}"
    );
}

#[test]
fn lists_and_dicts_hold_times() {
    let ty = schema();
    let shop = ty.type_def("shop").unwrap();
    assert_eq!(
        shop.field("slots").unwrap().kind.item().unwrap().kind,
        FieldKind::Time
    );
    let FieldKind::Dict { keys, .. } = &shop.field("hours").unwrap().kind else {
        panic!("hours is a dict");
    };
    assert!(keys.iter().all(|k| k.kind == FieldKind::Time));

    note(
        "slots = [\"09:00\", \"12:30:00\"]\n\ndict hours {\n  open  = \"08:00\"\n  close = \"17:30\"\n}\n\ndict breaks {\n  lunch = \"12:00\"\n}",
    )
    .unwrap();

    let issues = note(
        "slots = [\"09:00\", \"25:00\"]\n\ndict hours {\n  open = \"8:00\"\n}\n\ndict breaks {\n  lunch = \"noon\"\n}",
    )
    .unwrap_err();
    for at in ["slots[1]", "dict \"hours\".open", "dict \"breaks\".lunch"] {
        assert!(
            issues
                .iter()
                .any(|i| i.contains(at) && i.contains("is not a 24-hour time of day")),
            "{at}: {issues:?}"
        );
    }
}
