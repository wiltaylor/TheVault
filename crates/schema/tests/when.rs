//! A template tab's `when` condition (D14, D42): parsed and checked when the type is saved, and
//! unknown attributes inside templates tolerated as warnings.

use schema::{Issue, TypeDef};

const FIELDS: &str = r#"label = "Task"

field done {
  kind = "bool"
}

field status {
  kind    = "select"
  options = ["open", "closed"]
}

field due {
  kind = "date"
}

field span {
  kind = "date_range"
}

field owner {
  kind   = "reference"
  target = "task"
}

field tags {
  kind  = "list"
  items = "string"
}
"#;

/// The type with `tab` as the second tab of its default template.
fn with_tab(tab: &str) -> String {
    format!(
        "{FIELDS}\ntemplate default {{\n  tab \"Main\" {{\n    slot \"title\"\n  }}\n{tab}\n}}\n"
    )
}

fn text(issues: &[Issue]) -> Vec<String> {
    issues.iter().map(ToString::to_string).collect()
}

fn errors(source: &str) -> Vec<String> {
    match TypeDef::parse("task", source) {
        Ok(t) => panic!("the type parsed: {:?}", t.templates),
        Err(e) => text(&e.issues),
    }
}

#[test]
fn a_valid_when_parses_and_is_kept_on_the_tab() {
    for when in [
        "status == \"open\"",
        "!done && (due < today() + days(7) || has(tags, \"urgent\"))",
        "this.status != \"closed\"",
        "span.start >= \"2026-01-01\"",
        "any(tags, |t| t == \"x\") && !empty(title) && contains(title, \"a\")",
        "under(status, \"open\") || archived || created > \"2026-01-01\"",
    ] {
        let source = with_tab(&format!(
            "  tab \"Closing\" {{\n    when = {when:?}\n    slot \"status\"\n  }}"
        ));
        let t = TypeDef::parse("task", &source).unwrap_or_else(|e| panic!("{when}: {e}"));
        assert_eq!(t.warnings, vec![], "{when}");
        let tabs = &t.template("default").unwrap().tabs;
        assert_eq!(tabs[0].when, None);
        assert_eq!(tabs[1].when.as_deref(), Some(when));
    }
}

#[test]
fn a_when_that_does_not_parse_is_an_error_at_the_tab() {
    let issues = errors(&with_tab(
        "  tab \"Closing\" {\n    when = \"status == \"\n    slot \"status\"\n  }",
    ));
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(
        issues[0].starts_with("template default > tab \"Closing\".when: does not parse: 1:11:"),
        "{issues:?}"
    );
}

#[test]
fn a_when_naming_an_unknown_field_is_an_error_at_the_tab() {
    let issues = errors(&with_tab(
        "  tab \"Closing\" {\n    when = \"stage == \\\"x\\\" && done\"\n  }",
    ));
    assert_eq!(
        issues,
        ["template default > tab \"Closing\".when: `stage` is not a field of the type"]
    );
    let issues = errors(&with_tab(
        "  tab \"Closing\" {\n    when = \"this.stage\"\n  }",
    ));
    assert_eq!(
        issues,
        ["template default > tab \"Closing\".when: `stage` is not a field of the type"]
    );
}

#[test]
fn a_when_cannot_read_other_notes() {
    for (when, says) in [
        (
            "owner.done",
            "`owner.done` reads another note, which a tab condition cannot",
        ),
        (
            "has(linked_from(task, owner), this)",
            "linked_from() reads other notes, which a tab condition cannot",
        ),
        (
            "done.x",
            "`done` has no members: `.` reads a date range's start or end",
        ),
    ] {
        let issues = errors(&with_tab(&format!(
            "  tab \"Closing\" {{\n    when = {when:?}\n  }}"
        )));
        assert_eq!(
            issues,
            [format!("template default > tab \"Closing\".when: {says}")],
            "{when}"
        );
    }
}

#[test]
fn a_when_must_be_a_string() {
    let issues = errors(&with_tab("  tab \"Closing\" {\n    when = true\n  }"));
    assert!(
        issues
            .iter()
            .any(|i| i.starts_with("template default > tab \"Closing\".when: must be a string")),
        "{issues:?}"
    );
}

#[test]
fn the_card_template_takes_when_too() {
    let source = format!(
        "{FIELDS}\ntemplate default {{\n  tab \"Main\" {{\n    slot \"title\"\n  }}\n}}\n\ntemplate card {{\n  tab \"Open\" {{\n    when = \"!done\"\n    slot \"due\"\n  }}\n  tab \"Done\" {{\n    when = \"nope\"\n  }}\n}}\n"
    );
    assert_eq!(
        errors(&source),
        ["template card > tab \"Done\".when: `nope` is not a field of the type"]
    );
}

#[test]
fn unknown_attributes_inside_a_template_are_warnings_and_the_template_still_loads() {
    let source = format!(
        "{FIELDS}\ntemplate default {{\n  layout = \"wide\"\n  tab \"Main\" {{\n    icon = \"star\"\n    slot \"title\"\n    slot \"due\" {{\n      label = \"Due on\"\n      width = 2\n    }}\n    paragraph {{\n      text = \"Hello\"\n      tone = \"soft\"\n    }}\n  }}\n}}\n"
    );
    let t = TypeDef::parse("task", &source).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        text(&t.warnings),
        [
            "warning: template default.layout: is not an attribute of templates in this version; it is ignored",
            "warning: template default > tab \"Main\".icon: is not an attribute of tabs in this version; it is ignored",
            "warning: template default > tab \"Main\" > slot \"due\".width: is not an attribute of slots in this version; it is ignored",
            "warning: template default > tab \"Main\" > paragraph#3.tone: is not an attribute of template blocks in this version; it is ignored",
        ]
    );
    let tab = &t.template("default").unwrap().tabs[0];
    assert_eq!(tab.items.len(), 3, "every item still loads");
    assert!(matches!(
        &tab.items[1],
        schema::TabItem::Slot { field, label } if field == "due" && label.as_deref() == Some("Due on")
    ));
}

#[test]
fn an_unknown_block_in_a_template_stays_an_error() {
    let issues = errors(&with_tab("  tab \"Two\" {\n    sparkle {\n    }\n  }"));
    assert!(
        issues
            .iter()
            .any(|i| i.contains("sparkle") && !i.starts_with("warning:")),
        "{issues:?}"
    );
    // A known attribute with a malformed value is still an error.
    let issues = errors(&with_tab(
        "  tab \"Two\" {\n    slot \"due\" {\n      label = 3\n    }\n  }",
    ));
    assert!(
        issues.iter().any(|i| i
            .starts_with("template default > tab \"Two\" > slot \"due\".label: must be a string")),
        "{issues:?}"
    );
}
