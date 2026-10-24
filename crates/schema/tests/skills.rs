//! Skills S2: `SKILL.md` checked against every frontmatter constraint of the Agent Skills
//! specification.

use schema::{Issue, is_skill_name, parse_skill_md, split_frontmatter};

fn errors(issues: &[Issue]) -> Vec<(String, String)> {
    issues
        .iter()
        .filter(|i| !i.is_warning())
        .map(|i| (i.at.clone(), i.message.clone()))
        .collect()
}

fn errors_at(text: &str, folder: &str) -> Vec<String> {
    let (_, issues) = parse_skill_md(text, folder);
    errors(&issues).into_iter().map(|(at, _)| at).collect()
}

fn md(front: &str) -> String {
    format!("---\n{front}---\n# Body\n")
}

#[test]
fn a_full_skill_parses_with_no_issues() {
    let text = "---\nname: pdf-processing\ndescription: Extract PDF text, fill forms, merge files. Use when handling PDFs.\nlicense: Apache-2.0\ncompatibility: Requires Python 3.14+ and uv\nmetadata:\n  author: example-org\n  version: \"1.0\"\nallowed-tools: Bash(git:*) Bash(jq:*) Read\n---\n\n# PDF\n\nSee [the reference](references/REFERENCE.md).\n";
    let (skill, issues) = parse_skill_md(text, "pdf-processing");
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(skill.name.as_deref(), Some("pdf-processing"));
    assert_eq!(skill.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        skill.compatibility.as_deref(),
        Some("Requires Python 3.14+ and uv")
    );
    assert_eq!(skill.metadata["author"], "example-org");
    assert_eq!(skill.metadata["version"], "1.0");
    assert_eq!(
        skill.allowed_tools_list(),
        ["Bash(git:*)", "Bash(jq:*)", "Read"]
    );
    assert_eq!(
        skill.body,
        "\n# PDF\n\nSee [the reference](references/REFERENCE.md).\n"
    );
}

#[test]
fn missing_frontmatter_is_an_error() {
    assert_eq!(errors_at("# Just markdown\n", "a"), ["frontmatter"]);
    assert_eq!(errors_at("---\nname: a\n", "a"), ["frontmatter"]);
    assert_eq!(errors_at("", "a"), ["frontmatter"]);
    // Empty frontmatter: both required fields are missing.
    assert_eq!(errors_at("---\n---\n", "a"), ["name", "description"]);
    assert_eq!(errors_at("---\nnot yaml: [\n---\n", "a"), ["frontmatter"]);
    assert_eq!(errors_at("---\n- a\n- b\n---\n", "a"), ["frontmatter"]);
}

#[test]
fn frontmatter_splits_on_dash_lines_with_crlf_and_a_bom() {
    assert_eq!(
        split_frontmatter("\u{feff}---\r\nname: a\r\n---\r\nbody"),
        Some(("name: a\r\n", "body"))
    );
    assert_eq!(
        split_frontmatter("---\nname: a\n---"),
        Some(("name: a\n", ""))
    );
    assert_eq!(
        split_frontmatter("--- \nx: 1\n---  \nb"),
        Some(("x: 1\n", "b"))
    );
    assert_eq!(split_frontmatter("x\n---\n"), None);
}

#[test]
fn the_name_follows_every_rule() {
    for good in [
        "a",
        "pdf-processing",
        "data-analysis",
        "x1",
        &"a".repeat(64),
    ] {
        assert!(is_skill_name(good), "{good}");
        assert!(
            errors_at(&md(&format!("name: {good}\ndescription: d\n")), good).is_empty(),
            "{good}"
        );
    }
    for bad in [
        "PDF-Processing",
        "-pdf",
        "pdf-",
        "pdf--processing",
        "pdf_processing",
        "pdf processing",
        "café",
        &"a".repeat(65),
    ] {
        assert!(!is_skill_name(bad), "{bad}");
        let errs = errors_at(&md(&format!("name: \"{bad}\"\ndescription: d\n")), bad);
        assert_eq!(errs, ["name"], "{bad}");
    }
    assert_eq!(
        errors_at(&md("name: \"\"\ndescription: d\n"), "a"),
        ["name", "name"]
    );
    // Missing, or not a string.
    assert_eq!(errors_at(&md("description: d\n"), "a"), ["name"]);
    assert_eq!(errors_at(&md("name: 12\ndescription: d\n"), "12"), ["name"]);
}

#[test]
fn the_name_must_equal_the_folder() {
    let (_, issues) = parse_skill_md(&md("name: pdf\ndescription: d\n"), "pdf-tools");
    let errs = errors(&issues);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].0, "name");
    assert!(errs[0].1.contains("pdf-tools"), "{}", errs[0].1);
}

#[test]
fn the_description_is_required_and_at_most_1024_characters() {
    assert_eq!(errors_at(&md("name: a\n"), "a"), ["description"]);
    assert_eq!(
        errors_at(&md("name: a\ndescription: \"\"\n"), "a"),
        ["description"]
    );
    assert_eq!(
        errors_at(&md("name: a\ndescription: \"  \"\n"), "a"),
        ["description"]
    );
    assert_eq!(
        errors_at(&md("name: a\ndescription:\n"), "a"),
        ["description"]
    );
    let at_limit = "é".repeat(1024);
    assert!(errors_at(&md(&format!("name: a\ndescription: {at_limit}\n")), "a").is_empty());
    let over = "x".repeat(1025);
    assert_eq!(
        errors_at(&md(&format!("name: a\ndescription: {over}\n")), "a"),
        ["description"]
    );
    // A folded multi-line description is one string.
    let (skill, issues) = parse_skill_md(
        &md("name: a\ndescription: >\n  Does things.\n  Use when asked.\n"),
        "a",
    );
    assert!(issues.is_empty());
    assert_eq!(
        skill.description.as_deref(),
        Some("Does things. Use when asked.\n")
    );
}

#[test]
fn compatibility_is_1_to_500_characters() {
    let ok = "c".repeat(500);
    assert!(
        errors_at(
            &md(&format!("name: a\ndescription: d\ncompatibility: {ok}\n")),
            "a"
        )
        .is_empty()
    );
    let over = "c".repeat(501);
    assert_eq!(
        errors_at(
            &md(&format!("name: a\ndescription: d\ncompatibility: {over}\n")),
            "a"
        ),
        ["compatibility"]
    );
    assert_eq!(
        errors_at(&md("name: a\ndescription: d\ncompatibility: \"\"\n"), "a"),
        ["compatibility"]
    );
}

#[test]
fn metadata_is_a_map_of_strings() {
    assert_eq!(
        errors_at(&md("name: a\ndescription: d\nmetadata: text\n"), "a"),
        ["metadata"]
    );
    assert_eq!(
        errors_at(&md("name: a\ndescription: d\nmetadata:\n  - x\n"), "a"),
        ["metadata"]
    );
    let (skill, issues) = parse_skill_md(
        &md(
            "name: a\ndescription: d\nmetadata:\n  author: me\n  version: 1.0\n  n: 3\n  nested:\n    x: y\n",
        ),
        "a",
    );
    let at: Vec<String> = errors(&issues).into_iter().map(|(a, _)| a).collect();
    assert_eq!(at, ["metadata.version", "metadata.n", "metadata.nested"]);
    assert_eq!(skill.metadata.len(), 1);
    assert_eq!(skill.metadata["author"], "me");
    assert!(errors_at(&md("name: a\ndescription: d\nmetadata:\n"), "a").is_empty());
}

#[test]
fn the_optional_strings_must_be_strings() {
    assert_eq!(
        errors_at(&md("name: a\ndescription: d\nlicense: [MIT]\n"), "a"),
        ["license"]
    );
    assert_eq!(
        errors_at(&md("name: a\ndescription: d\nallowed-tools: [Read]\n"), "a"),
        ["allowed-tools"]
    );
}

#[test]
fn unknown_keys_are_kept_as_warnings() {
    let (skill, issues) = parse_skill_md(
        &md("name: a\ndescription: d\nversion: 2\ntags:\n  - one\n  - two\n"),
        "a",
    );
    assert!(errors(&issues).is_empty(), "{issues:?}");
    let warned: Vec<&str> = issues
        .iter()
        .filter(|i| i.is_warning())
        .map(|i| i.at.as_str())
        .collect();
    assert_eq!(warned, ["version", "tags"]);
    assert_eq!(skill.extra["version"], "2");
    assert_eq!(skill.extra["tags"], "- one\n- two");
}
