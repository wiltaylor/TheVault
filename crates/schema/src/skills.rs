//! `SKILL.md`, the file that makes a folder an Agent Skill (skills S2), checked against the Agent
//! Skills specification, <https://agentskills.io/specification>.
//!
//! The file is YAML frontmatter between two `---` lines, then Markdown instructions. The
//! frontmatter is parsed with a YAML parser and checked field by field:
//!
//! - `name`: required, 1 to 64 characters of `a-z`, `0-9` and `-`, not starting or ending with
//!   `-`, no `--`, and equal to the skill's folder name.
//! - `description`: required, 1 to 1024 characters.
//! - `license`: optional, a string.
//! - `compatibility`: optional, 1 to 500 characters.
//! - `metadata`: optional, a map of string keys to string values.
//! - `allowed-tools`: optional, a space-separated string. Experimental; it grants nothing here.
//!
//! Any other key is kept and reported as a warning. A skill with an error is invalid: it is listed
//! with its errors and offered to no agent.

use std::collections::BTreeMap;

use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

use crate::Issue;

/// The most characters a skill name may have.
pub const SKILL_NAME_MAX: usize = 64;

/// The most characters a skill description may have.
pub const SKILL_DESCRIPTION_MAX: usize = 1024;

/// The most characters a skill's `compatibility` may have.
pub const SKILL_COMPATIBILITY_MAX: usize = 500;

/// The frontmatter keys the specification defines.
pub const SKILL_KEYS: [&str; 6] = [
    "name",
    "description",
    "license",
    "compatibility",
    "metadata",
    "allowed-tools",
];

/// Whether `name` is a valid skill name: 1 to 64 characters of `a-z`, `0-9` and `-`, not
/// starting or ending with `-`, and no `--`.
pub fn is_skill_name(name: &str) -> bool {
    skill_name_problem(name).is_none()
}

/// Why `name` is not a valid skill name, or `None` when it is.
pub fn skill_name_problem(name: &str) -> Option<String> {
    let len = name.chars().count();
    if len == 0 || len > SKILL_NAME_MAX {
        return Some(format!(
            "must be 1 to {SKILL_NAME_MAX} characters; {name:?} has {len}"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Some(format!(
            "may hold only lowercase letters a-z, digits 0-9 and hyphens; {name:?} does not"
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Some(format!("must not start or end with a hyphen: {name:?}"));
    }
    if name.contains("--") {
        return Some(format!("must not hold two hyphens in a row: {name:?}"));
    }
    None
}

/// A parsed `SKILL.md`: the frontmatter fields the specification defines, the keys it does not,
/// and the Markdown body. A field the file does not set, or sets to a value of the wrong kind,
/// is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillMd {
    pub name: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    /// `metadata`, the string values only.
    pub metadata: BTreeMap<String, String>,
    /// `allowed-tools`, as written.
    pub allowed_tools: Option<String>,
    /// Every other frontmatter key, with its value as YAML text.
    pub extra: BTreeMap<String, String>,
    /// The Markdown after the frontmatter.
    pub body: String,
}

impl SkillMd {
    /// `allowed-tools` split on whitespace.
    pub fn allowed_tools_list(&self) -> Vec<&str> {
        self.allowed_tools
            .as_deref()
            .map(|t| t.split_whitespace().collect())
            .unwrap_or_default()
    }
}

/// Split `text` into its frontmatter and body: the lines between a first line `---` and the next
/// line `---`. `None` when the file does not start with frontmatter or never closes it.
pub fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first_end = text.find('\n')?;
    if text[..first_end].trim_end() != "---" {
        return None;
    }
    let rest = &text[first_end + 1..];
    let mut at = 0;
    while at <= rest.len() {
        let end = rest[at..].find('\n').map_or(rest.len(), |n| at + n);
        let line = &rest[at..end];
        if line.trim_end() == "---" {
            let body = rest.get(end + 1..).unwrap_or("");
            return Some((&rest[..at], body));
        }
        if end == rest.len() {
            break;
        }
        at = end + 1;
    }
    None
}

/// Parse and check `text` as the `SKILL.md` of the skill in folder `folder`. Returns what could
/// be read and every issue: errors make the skill invalid, warnings do not.
pub fn parse_skill_md(text: &str, folder: &str) -> (SkillMd, Vec<Issue>) {
    let mut skill = SkillMd::default();
    let mut issues = Vec::new();
    let Some((front, body)) = split_frontmatter(text) else {
        issues.push(Issue::new(
            "frontmatter",
            "SKILL.md must start with YAML frontmatter between two `---` lines",
        ));
        skill.body = text.to_owned();
        return (skill, issues);
    };
    skill.body = body.to_owned();
    let docs = match YamlLoader::load_from_str(front) {
        Ok(docs) => docs,
        Err(e) => {
            issues.push(Issue::new(
                "frontmatter",
                format!("the frontmatter is not valid YAML: {e}"),
            ));
            return (skill, issues);
        }
    };
    let hash = match docs.into_iter().next() {
        Some(Yaml::Hash(hash)) => hash,
        None | Some(Yaml::Null) => Default::default(),
        Some(_) => {
            issues.push(Issue::new(
                "frontmatter",
                "the frontmatter must be a YAML mapping of keys to values",
            ));
            return (skill, issues);
        }
    };
    for (key, value) in hash {
        let Some(key) = key.as_str().map(str::to_owned) else {
            issues.push(Issue::new(
                "frontmatter",
                format!(
                    "every frontmatter key must be a string; {} is not",
                    yaml_text(&key)
                ),
            ));
            continue;
        };
        match key.as_str() {
            "name" => skill.name = string_field(&key, &value, &mut issues),
            "description" => skill.description = string_field(&key, &value, &mut issues),
            "license" => skill.license = string_field(&key, &value, &mut issues),
            "compatibility" => skill.compatibility = string_field(&key, &value, &mut issues),
            "allowed-tools" => skill.allowed_tools = string_field(&key, &value, &mut issues),
            "metadata" => skill.metadata = metadata(&value, &mut issues),
            _ => {
                issues.push(Issue::warning(
                    key.clone(),
                    format!(
                        "{key:?} is not a field of the Agent Skills specification; it is kept and ignored"
                    ),
                ));
                skill.extra.insert(key, yaml_text(&value));
            }
        }
    }
    check(&skill, folder, &mut issues);
    (skill, issues)
}

fn check(skill: &SkillMd, folder: &str, issues: &mut Vec<Issue>) {
    match &skill.name {
        None if !issues.iter().any(|i| i.at == "name") => {
            issues.push(Issue::new("name", "is required"));
        }
        None => {}
        Some(name) => {
            if let Some(problem) = skill_name_problem(name) {
                issues.push(Issue::new("name", problem));
            }
            if name != folder {
                issues.push(Issue::new(
                    "name",
                    format!("must equal the skill's folder name {folder:?}; it is {name:?}"),
                ));
            }
        }
    }
    match &skill.description {
        None if !issues.iter().any(|i| i.at == "description") => {
            issues.push(Issue::new("description", "is required"));
        }
        None => {}
        Some(d) if d.trim().is_empty() => {
            issues.push(Issue::new("description", "must not be empty"));
        }
        Some(d) => {
            let len = d.chars().count();
            if len > SKILL_DESCRIPTION_MAX {
                issues.push(Issue::new(
                    "description",
                    format!("must be at most {SKILL_DESCRIPTION_MAX} characters; it has {len}"),
                ));
            }
        }
    }
    if let Some(c) = &skill.compatibility {
        let len = c.chars().count();
        if c.trim().is_empty() || len > SKILL_COMPATIBILITY_MAX {
            issues.push(Issue::new(
                "compatibility",
                format!("must be 1 to {SKILL_COMPATIBILITY_MAX} characters; it has {len}"),
            ));
        }
    }
}

/// The string value of `key`, or an issue when it is not a string.
fn string_field(key: &str, value: &Yaml, issues: &mut Vec<Issue>) -> Option<String> {
    match value {
        Yaml::String(s) => Some(s.clone()),
        other => {
            issues.push(Issue::new(
                key,
                format!(
                    "must be a string; it is {} (quote it to make it one)",
                    yaml_kind(other)
                ),
            ));
            None
        }
    }
}

fn metadata(value: &Yaml, issues: &mut Vec<Issue>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let hash = match value {
        Yaml::Hash(hash) => hash,
        Yaml::Null => return out,
        other => {
            issues.push(Issue::new(
                "metadata",
                format!(
                    "must be a map of string keys to string values; it is {}",
                    yaml_kind(other)
                ),
            ));
            return out;
        }
    };
    for (k, v) in hash {
        let Yaml::String(k) = k else {
            issues.push(Issue::new(
                "metadata",
                format!("every key must be a string; {} is not", yaml_text(k)),
            ));
            continue;
        };
        match v {
            Yaml::String(v) => {
                out.insert(k.clone(), v.clone());
            }
            other => issues.push(Issue::new(
                format!("metadata.{k}"),
                format!(
                    "must be a string; it is {} (quote it to make it one)",
                    yaml_kind(other)
                ),
            )),
        }
    }
    out
}

fn yaml_kind(v: &Yaml) -> &'static str {
    match v {
        Yaml::Real(_) => "a number",
        Yaml::Integer(_) => "a whole number",
        Yaml::String(_) => "a string",
        Yaml::Boolean(_) => "a boolean",
        Yaml::Array(_) => "a list",
        Yaml::Hash(_) => "a map",
        Yaml::Alias(_) => "an alias",
        Yaml::Null => "empty",
        Yaml::BadValue => "not a value",
    }
}

/// `v` written as YAML, without the document marker.
fn yaml_text(v: &Yaml) -> String {
    match v {
        Yaml::String(s) => return s.clone(),
        Yaml::Real(s) => return s.clone(),
        Yaml::Integer(n) => return n.to_string(),
        Yaml::Boolean(b) => return b.to_string(),
        Yaml::Null => return String::new(),
        _ => {}
    }
    let mut out = String::new();
    if YamlEmitter::new(&mut out).dump(v).is_err() {
        return String::new();
    }
    out.strip_prefix("---\n")
        .or_else(|| out.strip_prefix("---"))
        .unwrap_or(&out)
        .trim()
        .to_owned()
}
