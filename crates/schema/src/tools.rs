//! Script tool files (script-tools T2): `tools/<name>.wcl`, and `tools/<name>.wcl` inside a skill
//! (T4), name a script function an agent calls as a tool, as `docs/vault-format.md` "Tool files"
//! defines it.
//!
//! [`ScriptTool::parse`] checks one file alone: its vocabulary, the closed list of parameter
//! kinds, the script and function names, the hosts and secrets and the timeout. Whether the
//! script exists and compiles, and whether the name clashes with a built-in tool, is the
//! server's question.

use std::collections::HashSet;

use serde_json::{Value as Json, json};

use crate::doc::{Doc, Value};
use crate::encoding::{is_ident, is_name};
use crate::{Invalid, Issue};

/// The kinds a tool parameter may be.
pub const TOOL_PARAM_KINDS: [&str; 6] = ["string", "int", "float", "bool", "json", "list"];

/// The kinds a `list` parameter's items may be.
pub const TOOL_LIST_ITEM_KINDS: [&str; 4] = ["string", "int", "float", "bool"];

/// A tool's time limit when its file names none, in seconds.
pub const TOOL_TIMEOUT_DEFAULT: u64 = 30;

/// The longest time limit a tool may ask for, in seconds.
pub const TOOL_TIMEOUT_MAX: u64 = 300;

/// The longest tool description, in characters.
pub const TOOL_DESCRIPTION_MAX: usize = 4_000;

/// The top-level fields of a tool file.
const FIELDS: [&str; 6] = [
    "description",
    "script",
    "function",
    "allow_domains",
    "secrets",
    "timeout",
];

/// The fields of a `param` block.
const PARAM_FIELDS: [&str; 4] = ["kind", "of", "description", "required"];

/// One tool file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptTool {
    /// The file stem.
    pub name: String,
    /// What the model reads about the tool.
    pub description: String,
    /// In file order.
    pub params: Vec<ToolParam>,
    /// The script's name: `scripts/<script>.ws`, or the skill's own `scripts/<script>.ws`.
    pub script: String,
    /// The exported function, called with one argument: a map of the parameters.
    pub function: String,
    /// Hosts the `http` module may reach; `"*"` is any host.
    pub allow_domains: Vec<String>,
    /// Secrets the `secrets` module may read and write.
    pub secrets: Vec<String>,
    /// The wall-clock limit in seconds.
    pub timeout: u64,
}

/// A `param <name> { ... }` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolParam {
    pub name: String,
    /// One of [`TOOL_PARAM_KINDS`].
    pub kind: String,
    /// For a `list`: one of [`TOOL_LIST_ITEM_KINDS`]. `None` for any other kind.
    pub of: Option<String>,
    pub description: Option<String>,
    pub required: bool,
}

impl ToolParam {
    /// The parameter's JSON schema, nullable when it is optional. A `json` parameter is JSON text,
    /// which strict tool schemas can describe.
    pub fn schema(&self) -> Json {
        let base = |kind: &str| match kind {
            "int" => "integer",
            "float" => "number",
            "bool" => "boolean",
            _ => "string",
        };
        let mut s = if self.kind == "list" {
            let of = self.of.as_deref().unwrap_or("string");
            json!({ "type": "array", "items": { "type": base(of) } })
        } else {
            json!({ "type": base(&self.kind) })
        };
        if !self.required {
            let t = s["type"].clone();
            s["type"] = json!([t, "null"]);
        }
        let mut description = self.description.clone().unwrap_or_default();
        if self.kind == "json" {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str("JSON text.");
        }
        if !self.required {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str("Optional: null to leave it out.");
        }
        if !description.is_empty() {
            s["description"] = json!(description);
        }
        s
    }

    /// The JSON form, as the tool routes answer it.
    pub fn to_json(&self) -> Json {
        json!({
            "name": self.name,
            "kind": self.kind,
            "of": self.of,
            "description": self.description,
            "required": self.required,
        })
    }
}

impl ScriptTool {
    /// Read tool `name` from the text of its file at `file`, such as `tools/<name>.wcl`. Every
    /// problem is reported at once.
    pub fn parse_at(name: &str, source: &str, file: &str) -> Result<ScriptTool, Invalid> {
        let doc = Doc::parse(source, file)?;
        ScriptTool::from_doc(name, &doc).map_err(|issues| Invalid {
            file: file.to_owned(),
            issues,
        })
    }

    /// Read tool `name` from the text of `tools/<name>.wcl`.
    pub fn parse(name: &str, source: &str) -> Result<ScriptTool, Invalid> {
        ScriptTool::parse_at(name, source, &format!("tools/{name}.wcl"))
    }

    pub fn from_doc(name: &str, doc: &Doc) -> Result<ScriptTool, Vec<Issue>> {
        let mut issues = Vec::new();
        if !is_name(name) {
            issues.push(Issue::new(
                "name",
                format!(
                    "{name:?} is not a valid tool name: use [a-z][a-z0-9_]*, at most 64 characters"
                ),
            ));
        }
        let mut seen = HashSet::new();
        for (field, _) in &doc.fields {
            if !seen.insert(field.as_str()) {
                issues.push(Issue::new(field, "appears more than once"));
            } else if !FIELDS.contains(&field.as_str()) {
                issues.push(Issue::new(
                    field,
                    "is not a tool field: use description, script, function, allow_domains, secrets, timeout or param <name> { ... }",
                ));
            }
        }
        let mut r = Reader {
            doc,
            issues: &mut issues,
        };
        let description = r.required("description").unwrap_or_default();
        if r.doc.field("description").is_some() && description.trim().is_empty() {
            r.issue("description", "must not be empty");
        } else if description.chars().count() > TOOL_DESCRIPTION_MAX {
            r.issue(
                "description",
                format!("is longer than {TOOL_DESCRIPTION_MAX} characters"),
            );
        }
        let script = r.required("script").unwrap_or_default();
        if !script.is_empty() && !is_name(&script) {
            r.issue("script", format!("{script:?} is not a script name"));
        }
        let function = r.required("function").unwrap_or_default();
        if !function.is_empty() && !is_ident(&function) {
            r.issue("function", format!("{function:?} is not a function name"));
        }
        let allow_domains = r.strings("allow_domains");
        for d in &allow_domains {
            if !is_host(d) {
                r.issue(
                    "allow_domains",
                    format!("{d:?} is not a host name, or \"*\" for any host"),
                );
            }
        }
        let secrets = r.strings("secrets");
        for s in &secrets {
            if !is_name(s) {
                r.issue("secrets", format!("{s:?} is not a secret name"));
            }
        }
        let timeout = match doc.field("timeout") {
            None => TOOL_TIMEOUT_DEFAULT,
            Some(Value::Int(n)) if (1..=TOOL_TIMEOUT_MAX as i64).contains(n) => *n as u64,
            Some(_) => {
                r.issue(
                    "timeout",
                    format!("must be a whole number of seconds from 1 to {TOOL_TIMEOUT_MAX}"),
                );
                TOOL_TIMEOUT_DEFAULT
            }
        };
        let mut params = Vec::new();
        let mut names = HashSet::new();
        for b in &doc.blocks {
            if b.kind != "param" {
                r.issue(&b.name(), "is not a tool block: use param <name> { ... }");
                continue;
            }
            let pname = b.label().unwrap_or_default().to_owned();
            let at = format!("param {pname}");
            if b.labels.len() != 1 || !is_name(&pname) {
                r.issue(
                    &at,
                    "the label must be one parameter name: [a-z][a-z0-9_]*, at most 64 characters",
                );
                continue;
            }
            if !names.insert(pname.clone()) {
                r.issue(&at, "appears more than once");
                continue;
            }
            if let Some(p) = r.param(&pname, &b.body, &at) {
                params.push(p);
            }
        }
        if issues.iter().any(|i| !i.is_warning()) {
            return Err(issues);
        }
        Ok(ScriptTool {
            name: name.to_owned(),
            description,
            params,
            script,
            function,
            allow_domains,
            secrets,
            timeout,
        })
    }

    /// The tool's input schema: an object of every parameter, each required by name and the
    /// optional ones nullable, with nothing else, as strict tool schemas need.
    pub fn input_schema(&self) -> Json {
        let mut properties = serde_json::Map::new();
        for p in &self.params {
            properties.insert(p.name.clone(), p.schema());
        }
        let required: Vec<&str> = self.params.iter().map(|p| p.name.as_str()).collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    /// The JSON form, as the tool routes answer it.
    pub fn to_json(&self) -> Json {
        json!({
            "name": self.name,
            "description": self.description,
            "script": self.script,
            "function": self.function,
            "params": self.params.iter().map(ToolParam::to_json).collect::<Vec<_>>(),
            "allow_domains": self.allow_domains,
            "secrets": self.secrets,
            "timeout": self.timeout,
        })
    }

    /// The function's one argument for a call with `input`: a map of every parameter, an absent
    /// optional one as `null` and a `json` one parsed from its text. Every problem is in the
    /// error, so the model can fix the call.
    pub fn arguments(&self, input: &Json) -> Result<Json, String> {
        let empty = serde_json::Map::new();
        let given = match input {
            Json::Object(o) => o,
            Json::Null => &empty,
            _ => return Err("the input must be an object of the parameters".to_owned()),
        };
        let mut problems = Vec::new();
        for k in given.keys() {
            if !self.params.iter().any(|p| &p.name == k) {
                problems.push(format!("{k} is not a parameter of this tool"));
            }
        }
        let mut out = serde_json::Map::new();
        for p in &self.params {
            let v = given.get(&p.name).cloned().unwrap_or(Json::Null);
            if v.is_null() {
                if p.required {
                    problems.push(format!("{} is required", p.name));
                }
                out.insert(p.name.clone(), Json::Null);
                continue;
            }
            match p.check(v) {
                Ok(v) => {
                    out.insert(p.name.clone(), v);
                }
                Err(e) => problems.push(e),
            }
        }
        if problems.is_empty() {
            Ok(Json::Object(out))
        } else {
            Err(problems.join("; "))
        }
    }
}

impl ToolParam {
    /// `v` as this parameter takes it, or why not.
    fn check(&self, v: Json) -> Result<Json, String> {
        let bad = |what: &str| format!("{} must be {what}", self.name);
        let scalar = |kind: &str, v: Json| -> Result<Json, String> {
            match kind {
                "string" if v.is_string() => Ok(v),
                "int" if v.is_i64() || v.is_u64() => Ok(v),
                // A whole float the model sent for an integer.
                "int"
                    if v.as_f64()
                        .is_some_and(|f| f.fract() == 0.0 && f.abs() < 9e15) =>
                {
                    Ok(json!(v.as_f64().unwrap_or_default() as i64))
                }
                "float" if v.is_number() => Ok(v),
                "bool" if v.is_boolean() => Ok(v),
                "string" => Err(bad("a string")),
                "int" => Err(bad("an integer")),
                "float" => Err(bad("a number")),
                _ => Err(bad("true or false")),
            }
        };
        match self.kind.as_str() {
            "json" => match v {
                Json::String(text) => serde_json::from_str(&text)
                    .map_err(|e| format!("{} is not JSON text: {e}", self.name)),
                other => Ok(other),
            },
            "list" => {
                let of = self.of.as_deref().unwrap_or("string");
                match v {
                    Json::Array(items) => items
                        .into_iter()
                        .map(|i| scalar(of, i))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Json::Array),
                    _ => Err(bad("a list")),
                }
            }
            kind => scalar(kind, v),
        }
    }
}

/// A DNS name or an IP address, without a scheme, port or path, or `*` for any host.
fn is_host(s: &str) -> bool {
    s == "*"
        || !s.is_empty()
            && s.len() <= 253
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
            && !s.starts_with(['.', '-'])
}

struct Reader<'a> {
    doc: &'a Doc,
    issues: &'a mut Vec<Issue>,
}

impl Reader<'_> {
    fn issue(&mut self, at: &str, message: impl Into<String>) {
        self.issues.push(Issue::new(at, message));
    }

    fn string_in(&mut self, doc: &Doc, field: &str, at: &str) -> Option<String> {
        match doc.field(field)? {
            Value::Str(s) => Some(s.clone()),
            v => {
                let msg = format!("must be a string, not {}", v.describe());
                self.issue(at, msg);
                None
            }
        }
    }

    fn required(&mut self, field: &str) -> Option<String> {
        if self.doc.field(field).is_none() {
            self.issue(field, "is required");
        }
        self.string_in(self.doc, field, field)
    }

    fn strings(&mut self, field: &str) -> Vec<String> {
        match self.doc.field(field) {
            None => Vec::new(),
            Some(Value::List(items)) => {
                let mut out = Vec::new();
                for v in items {
                    match v {
                        Value::Str(s) => out.push(s.clone()),
                        v => {
                            let msg = format!("must hold strings only, not {}", v.describe());
                            self.issue(field, msg);
                        }
                    }
                }
                out
            }
            Some(v) => {
                let msg = format!("must be a list of strings, not {}", v.describe());
                self.issue(field, msg);
                Vec::new()
            }
        }
    }

    fn param(&mut self, name: &str, d: &Doc, at: &str) -> Option<ToolParam> {
        let before = self.issues.len();
        for b in &d.blocks {
            self.issue(&format!("{at} > {}", b.name()), "a param holds fields only");
        }
        let mut seen = HashSet::new();
        for (field, _) in &d.fields {
            if !seen.insert(field.as_str()) {
                self.issue(&format!("{at}.{field}"), "appears more than once");
            } else if !PARAM_FIELDS.contains(&field.as_str()) {
                self.issue(
                    &format!("{at}.{field}"),
                    "is not a param field: use kind, of, description or required",
                );
            }
        }
        let kind_at = format!("{at}.kind");
        let kind = match self.string_in(d, "kind", &kind_at) {
            None if d.field("kind").is_none() => {
                self.issue(&kind_at, "is required");
                None
            }
            None => None,
            Some(k) if TOOL_PARAM_KINDS.contains(&k.as_str()) => Some(k),
            Some(k) => {
                self.issue(
                    &kind_at,
                    format!("{k:?} is not a kind: use {}", TOOL_PARAM_KINDS.join(", ")),
                );
                None
            }
        };
        let of_at = format!("{at}.of");
        let of = self.string_in(d, "of", &of_at);
        match (&kind, &of) {
            (Some(k), Some(_)) if k != "list" => {
                self.issue(&of_at, "belongs to a list parameter");
            }
            (_, Some(o)) if !TOOL_LIST_ITEM_KINDS.contains(&o.as_str()) => {
                self.issue(
                    &of_at,
                    format!(
                        "{o:?} is not a list item kind: use {}",
                        TOOL_LIST_ITEM_KINDS.join(", ")
                    ),
                );
            }
            _ => {}
        }
        let description = self.string_in(d, "description", &format!("{at}.description"));
        let required = match d.field("required") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(v) => {
                let msg = format!("must be true or false, not {}", v.describe());
                self.issue(&format!("{at}.required"), msg);
                false
            }
        };
        if self.issues.len() != before {
            return None;
        }
        let kind = kind?;
        Some(ToolParam {
            name: name.to_owned(),
            of: (kind == "list").then(|| of.unwrap_or_else(|| "string".to_owned())),
            kind,
            description,
            required,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNT: &str = r#"description = "Count tagged tasks."
script = "count_tagged"
function = "run"
allow_domains = ["api.example.com"]
secrets = ["example_key"]
timeout = 60

param tag {
  kind = "string"
  description = "The tag."
  required = true
}

param limit {
  kind = "int"
}

param names {
  kind = "list"
  of = "int"
}

param extra {
  kind = "json"
}
"#;

    #[test]
    fn a_tool_file_reads_every_field_and_param() {
        let t = ScriptTool::parse("count_tagged", COUNT).unwrap();
        assert_eq!(t.description, "Count tagged tasks.");
        assert_eq!(t.script, "count_tagged");
        assert_eq!(t.function, "run");
        assert_eq!(t.allow_domains, ["api.example.com"]);
        assert_eq!(t.secrets, ["example_key"]);
        assert_eq!(t.timeout, 60);
        let names: Vec<_> = t.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["tag", "limit", "names", "extra"]);
        assert!(t.params[0].required);
        assert_eq!(t.params[2].of.as_deref(), Some("int"));
        assert_eq!(t.params[1].of, None);
    }

    #[test]
    fn the_schema_lists_every_param_and_makes_optional_ones_nullable() {
        let t = ScriptTool::parse("count_tagged", COUNT).unwrap();
        let s = t.input_schema();
        assert_eq!(s["required"], json!(["tag", "limit", "names", "extra"]));
        assert_eq!(s["additionalProperties"], false);
        assert_eq!(s["properties"]["tag"]["type"], "string");
        assert_eq!(s["properties"]["limit"]["type"], json!(["integer", "null"]));
        assert_eq!(
            s["properties"]["names"]["type"],
            json!(["array", "null"]),
            "{s}"
        );
        assert_eq!(s["properties"]["names"]["items"]["type"], "integer");
        assert_eq!(s["properties"]["extra"]["type"], json!(["string", "null"]));
        assert!(
            s["properties"]["extra"]["description"]
                .as_str()
                .unwrap()
                .contains("JSON text")
        );
    }

    #[test]
    fn arguments_fill_absent_params_with_null_and_parse_json() {
        let t = ScriptTool::parse("count_tagged", COUNT).unwrap();
        let a = t
            .arguments(&json!({ "tag": "x", "extra": "{\"a\": 1}", "names": [1, 2.0] }))
            .unwrap();
        assert_eq!(
            a,
            json!({ "tag": "x", "limit": null, "names": [1, 2], "extra": { "a": 1 } })
        );
        let e = t
            .arguments(&json!({ "limit": "many", "other": 1 }))
            .unwrap_err();
        assert!(e.contains("tag is required"), "{e}");
        assert!(e.contains("limit must be an integer"), "{e}");
        assert!(e.contains("other is not a parameter"), "{e}");
    }

    #[test]
    fn every_problem_is_reported() {
        let src = r#"script = "Bad Name"
function = "1x"
timeout = 900
colour = "red"
allow_domains = ["https://x.com"]

param tag {
  kind = "text"
}

param tag {
  kind = "string"
}

param items {
  kind = "string"
  of = "int"
}

thing {
}
"#;
        let e = ScriptTool::parse("count", src).unwrap_err();
        let at: Vec<&str> = e.issues.iter().map(|i| i.at.as_str()).collect();
        for want in [
            "description",
            "script",
            "function",
            "timeout",
            "colour",
            "allow_domains",
            "param tag.kind",
            "param tag",
            "param items.of",
            "thing",
        ] {
            assert!(at.contains(&want), "{want} in {at:?}");
        }
        assert!(ScriptTool::parse("Bad", COUNT).is_err());
    }
}
