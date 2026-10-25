//! The template-only `period_calendar` block of the planner's period note types.

use std::collections::HashMap;

use schema::{Issue, Schema, TypeDef};

fn text(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

/// A type with a `date` field of `kind` (none when empty) and `tab` as its default tab's body.
fn ty(kind: &str, tab: &str) -> String {
    let date = if kind.is_empty() {
        String::new()
    } else {
        format!("field date {{\n  kind = \"{kind}\"\n  required = true\n}}\n")
    };
    format!(
        "label = \"Week\"\n{date}field body {{\n  kind = \"content\"\n}}\ntemplate default {{\n  tab \"Week\" {{\n{tab}    slot \"body\"\n  }}\n}}\n"
    )
}

fn issues(source: &str) -> Vec<String> {
    match TypeDef::parse("week", source) {
        Ok(t) => panic!("type parsed: {t:?}"),
        Err(e) => text(&e.issues),
    }
}

#[test]
fn a_template_shows_a_calendar_of_its_notes_period() {
    for period in ["week", "month", "quarter", "year"] {
        let src = ty(
            "date",
            &format!("    period_calendar {{\n      period = \"{period}\"\n    }}\n"),
        );
        TypeDef::parse("week", &src).unwrap_or_else(|e| panic!("{period}: {e:?}"));
    }
    let src = ty(
        "date",
        "    period_calendar {\n      period = \"month\"\n      query = \"from task\"\n      date = \"due\"\n    }\n",
    );
    TypeDef::parse("week", &src).unwrap();
    // In a template's column too.
    let src = ty(
        "date",
        "    columns {\n      column {\n        period_calendar {\n          period = \"week\"\n        }\n      }\n      column {\n        slot \"title\"\n      }\n    }\n",
    );
    TypeDef::parse("week", &src).unwrap();
}

#[test]
fn every_shipped_period_type_holds_one() {
    for (name, src) in [
        (
            "weeklynote",
            include_str!("../../writer/defaults/weeklynote.wcl"),
        ),
        (
            "monthlynote",
            include_str!("../../writer/defaults/monthlynote.wcl"),
        ),
        (
            "quarterlynote",
            include_str!("../../writer/defaults/quarterlynote.wcl"),
        ),
        (
            "yearlynote",
            include_str!("../../writer/defaults/yearlynote.wcl"),
        ),
    ] {
        let t = TypeDef::parse(name, src).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let json = format!("{t:?}");
        assert!(json.contains("period_calendar"), "{name}");
    }
}

#[test]
fn a_period_calendar_needs_a_known_period_and_nothing_else() {
    let found = issues(&ty("date", "    period_calendar {\n    }\n"));
    assert!(
        found
            .iter()
            .any(|i| i.ends_with("period_calendar#1.period: is required")),
        "{found:?}"
    );
    let found = issues(&ty(
        "date",
        "    period_calendar {\n      period = \"fortnight\"\n    }\n",
    ));
    assert!(
        found.iter().any(|i| i.contains("period_calendar#1.period")),
        "{found:?}"
    );
    let found = issues(&ty(
        "date",
        "    period_calendar {\n      period = \"week\"\n      query = 3\n    }\n",
    ));
    assert!(
        found.iter().any(|i| i.contains("period_calendar#1.query")),
        "{found:?}"
    );
    let found = issues(&ty(
        "date",
        "    period_calendar {\n      period = \"week\"\n      paragraph {\n        text = \"x\"\n      }\n    }\n",
    ));
    assert!(
        found.iter().any(|i| i.contains("period_calendar#1")),
        "{found:?}"
    );
}

#[test]
fn a_period_calendar_reads_the_notes_own_date_field() {
    let block = "    period_calendar {\n      period = \"week\"\n    }\n";
    let found = issues(&ty("", block));
    assert!(
        found.iter().any(|i| i.contains("the type has none")),
        "{found:?}"
    );
    let found = issues(&ty("string", block));
    assert!(
        found
            .iter()
            .any(|i| i.contains("reads the note's `date` field, which is")),
        "{found:?}"
    );
}

#[test]
fn a_notes_content_cannot_hold_a_period_calendar() {
    let schema = Schema::from_sources(&[("week", &ty("date", ""))], &[]).unwrap();
    let source = "title = \"A week\"\ndate = \"2026-10-19\"\ncontent body {\n  period_calendar {\n    period = \"week\"\n  }\n}\n";
    let err = schema
        .validate_note_source("week", source, "note.wcl", &HashMap::new())
        .unwrap_err();
    let found = text(&err.issues);
    assert!(
        found.iter().any(|i| i.contains("period_calendar")),
        "{found:?}"
    );
}
