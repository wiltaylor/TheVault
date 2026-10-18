//! `toggle` and `admonition` content blocks: their items, the closed sets of admonition kinds,
//! colours and icons, and the blocks nested in them, in content, canvas cards and templates.

use std::collections::HashMap;

use schema::{ADMONITION_KINDS, Issue, Schema, TypeDef, admonition_preset, is_icon};

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

fn para(t: &str) -> String {
    format!("paragraph {{\n  text = \"{t}\"\n}}\n")
}

#[test]
fn a_toggle_has_a_summary_an_optional_level_and_any_nested_blocks() {
    check("toggle {\n  text = \"Plain **summary**\"\n}").unwrap();
    check(&format!(
        "toggle {{\n  level = 2\n  text = \"Details\"\n{}list {{\n  style = \"bullet\"\n  item {{\n    text = \"x\"\n  }}\n}}\n}}",
        para("inside")
    ))
    .unwrap();
    for level in 0..=3 {
        check(&format!("toggle {{\n  level = {level}\n  text = \"x\"\n}}")).unwrap();
    }
    // Toggles and admonitions nest in each other, to any depth.
    check(&format!(
        "toggle {{\n  text = \"a\"\n  admonition {{\n    kind = \"tip\"\n    toggle {{\n      text = \"b\"\n{}    }}\n  }}\n}}",
        para("deep")
    ))
    .unwrap();
}

#[test]
fn a_toggle_needs_its_text_and_a_level_from_0_to_3() {
    let missing = check("toggle {\n  level = 1\n}").unwrap_err();
    assert!(has(&missing, "toggle#1.text: is required"), "{missing:?}");
    let level = check("toggle {\n  level = 4\n  text = \"x\"\n}").unwrap_err();
    assert!(
        has(&level, "toggle#1.level: must be from 0 to 3"),
        "{level:?}"
    );
    let odd = check("toggle {\n  text = \"x\"\n  open = true\n}").unwrap_err();
    assert!(
        has(&odd, "toggle#1.open: is not an item of this block"),
        "{odd:?}"
    );
}

#[test]
fn nested_blocks_are_checked_as_content_blocks() {
    let bad = check("toggle {\n  text = \"x\"\n  heading {\n    level = 9\n    text = \"y\"\n  }\n  stroke {\n  }\n}").unwrap_err();
    assert!(
        has(&bad, "toggle#1 > heading#1.level: must be from 1 to 3"),
        "{bad:?}"
    );
    assert!(
        has(
            &bad,
            "toggle#1 > stroke#2: \"stroke\" is not a content block kind"
        ),
        "{bad:?}"
    );
    let deep = check(
        "admonition {\n  kind = \"note\"\n  toggle {\n    text = \"x\"\n    quote {\n    }\n  }\n}",
    )
    .unwrap_err();
    assert!(
        has(&deep, "admonition#1 > toggle#1 > quote#1.text: is required"),
        "{deep:?}"
    );
}

#[test]
fn layouts_stay_at_the_top_toggles_and_admonitions_hold_no_columns() {
    let cols = "columns {\n  column {\n  }\n  column {\n  }\n}\n";
    let issues = check(&format!("toggle {{\n  text = \"x\"\n{cols}}}")).unwrap_err();
    assert!(
        has(
            &issues,
            "toggle#1 > columns#1: a toggle cannot hold a columns block"
        ),
        "{issues:?}"
    );
    let issues = check(&format!("admonition {{\n  kind = \"info\"\n{cols}}}")).unwrap_err();
    assert!(
        has(&issues, "an admonition cannot hold a columns block"),
        "{issues:?}"
    );
    // A column holds toggles and admonitions.
    check(&format!(
        "columns {{\n  column {{\n    toggle {{\n      text = \"a\"\n{}    }}\n  }}\n  column {{\n    admonition {{\n      kind = \"danger\"\n    }}\n  }}\n}}",
        para("b")
    ))
    .unwrap();
}

#[test]
fn every_admonition_kind_with_its_optional_items() {
    for kind in ADMONITION_KINDS {
        check(&format!(
            "admonition {{\n  kind = \"{kind}\"\n{}}}",
            para("x")
        ))
        .unwrap();
    }
    check(&format!(
        "admonition {{\n  kind = \"warning\"\n  title = \"Mind **the** gap\"\n  colour = \"pink\"\n  icon = \"flame\"\n  collapsible = true\n{}}}",
        para("x")
    ))
    .unwrap();
}

#[test]
fn an_unknown_kind_colour_or_icon_is_refused_with_a_message() {
    let kind = check("admonition {\n  kind = \"shout\"\n}").unwrap_err();
    assert!(
        has(
            &kind,
            "admonition#1.kind: \"shout\" is not one of note, info, tip, success, warning, danger, question, quote"
        ),
        "{kind:?}"
    );
    let missing = check("admonition {\n  title = \"x\"\n}").unwrap_err();
    assert!(
        has(&missing, "admonition#1.kind: is required"),
        "{missing:?}"
    );
    let colour = check("admonition {\n  kind = \"tip\"\n  colour = \"teal\"\n}").unwrap_err();
    assert!(
        has(
            &colour,
            "admonition#1.colour: \"teal\" is not a palette colour; use one of gray, brown"
        ),
        "{colour:?}"
    );
    let icon = check("admonition {\n  kind = \"tip\"\n  icon = \"no-such-icon\"\n}").unwrap_err();
    assert!(
        has(
            &icon,
            "admonition#1.icon: \"no-such-icon\" is not an icon name"
        ),
        "{icon:?}"
    );
    let flag = check("admonition {\n  kind = \"tip\"\n  collapsible = \"yes\"\n}").unwrap_err();
    assert!(
        has(&flag, "admonition#1.collapsible: must be true or false"),
        "{flag:?}"
    );
}

#[test]
fn each_preset_has_a_palette_colour_and_a_known_icon() {
    let want = [
        ("note", "gray", "pencil", "Note"),
        ("info", "blue", "info", "Info"),
        ("tip", "green", "lightbulb", "Tip"),
        ("success", "green", "circle-check", "Success"),
        ("warning", "orange", "triangle-alert", "Warning"),
        ("danger", "red", "octagon-alert", "Danger"),
        ("question", "purple", "circle-question-mark", "Question"),
        ("quote", "brown", "quote", "Quote"),
    ];
    for (kind, colour, icon, label) in want {
        let p = admonition_preset(kind).unwrap();
        assert_eq!((p.colour, p.icon, p.label), (colour, icon, label), "{kind}");
        assert!(is_icon(p.icon), "{icon}");
    }
    assert!(admonition_preset("shout").is_none());
}

#[test]
fn a_card_holds_toggles_but_no_canvas_inside_them() {
    let card = |inner: &str| {
        format!(
            "}}\ncanvas plan {{\n  viewport {{\n    x = 0.0\n    y = 0.0\n    zoom = 1.0\n  }}\n  node c {{\n    kind = \"card\"\n    x = 0.0\n    y = 0.0\n    width = 200.0\n    height = 100.0\n    toggle {{\n      text = \"t\"\n{inner}    }}\n  }}\n"
        )
    };
    // `check` wraps the body in `content body { ... }`; the card closes it and opens the canvas.
    check(&card(&para("ok"))).unwrap();
    let canvas = "admonition {\n  kind = \"note\"\n  canvas {\n    viewport {\n      x = 0.0\n      y = 0.0\n      zoom = 1.0\n    }\n  }\n}\n";
    let issues = check(&card(canvas)).unwrap_err();
    assert!(
        has(
            &issues,
            "toggle#1 > admonition#1 > canvas#1: a card cannot hold a canvas"
        ),
        "{issues:?}"
    );
}

#[test]
fn template_checks_walk_into_toggles_and_admonitions() {
    let tab = "    toggle {\n      text = \"About {{nothing}}\"\n      admonition {\n        kind = \"info\"\n        title = \"Due {{missing}}\"\n      }\n    }\n";
    let source = format!("label = \"T\"\ntemplate default {{\n  tab \"Main\" {{\n{tab}  }}\n}}\n");
    let issues = match TypeDef::parse("t", &source) {
        Ok(t) => panic!("type parsed: {t:?}"),
        Err(e) => text(&e.issues),
    };
    assert!(
        has(&issues, "toggle#1.text: `{{nothing}}` names no field"),
        "{issues:?}"
    );
    assert!(
        has(&issues, "admonition.title: `{{missing}}` names no field"),
        "{issues:?}"
    );
}

#[test]
fn placeholders_in_a_toggle_summary_and_an_admonition_title_are_checked() {
    let issues = check(
        "toggle {\n  text = \"[[+nosuch|A]]\"\n  admonition {\n    kind = \"tip\"\n    title = \"[[+gone|B]]\"\n  }\n}",
    )
    .unwrap_err();
    assert!(has(&issues, "toggle#1.text"), "{issues:?}");
    assert!(has(&issues, "admonition#1.title"), "{issues:?}");
}
