//! Agent files (agents G1, G4): `agents/<name>.wcl` names an agent's model, system prompt,
//! permissions and triggers, as `docs/vault-format.md` "Agent files" defines it.
//!
//! [`Agent::parse`] checks one file alone: its vocabulary, the closed lists (providers, efforts,
//! confirmation and internet switches, trigger operations), each schedule as the scheduler reads
//! a task's, and each `where` condition in the query language. [`Agent::check`] checks it against
//! the vault's types: every type `permissions.types` or a trigger names must exist.
//! [`Agent::to_wcl`] writes an agent back as WCL, for a client that edits the parsed form.

use std::collections::HashSet;
use std::str::FromStr;

use chrono::TimeZone;
use chrono_tz::Tz;
use croner::Cron;

use crate::blocks::{Check, Form};
use crate::doc::{Doc, DocBlock, Value};
use crate::encoding::is_name;
use crate::{Invalid, Issue, Schema};

/// The agent the vault ships, `agents/assistant.wcl`: the chat panel's default (agents G6). It
/// cannot be deleted or disabled.
pub const ASSISTANT_AGENT: &str = "assistant";

/// The providers an agent may name. Absent means the Settings > AI default.
pub const AGENT_PROVIDERS: [&str; 3] = ["anthropic", "openai", "chatgpt"];

/// The efforts an agent may ask for, lowest first.
pub const AGENT_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The confirmation switches of `permissions.confirm` (ai D22).
pub const CONFIRM_SWITCHES: [&str; 6] = [
    "notes",
    "archive",
    "delete",
    "types",
    "downloads",
    "taxonomies",
];

/// The internet switches of `permissions.web` (ai D28).
pub const WEB_SWITCHES: [&str; 3] = ["search", "read", "download"];

/// The note changes a trigger's `on` may name (agents G10).
pub const TRIGGER_OPS: [&str; 3] = ["create", "update", "delete"];

/// The longest `system_prompt` or trigger `prompt`, in characters, as the chat prompt's limit
/// (ai D17).
pub const AGENT_PROMPT_MAX: usize = 32_000;

/// The top-level fields of an agent file.
const FIELDS: [&str; 9] = [
    "label",
    "description",
    "icon",
    "enabled",
    "provider",
    "model",
    "effort",
    "system_prompt",
    // Blocks, listed so a field of the name reads as misplaced rather than unknown.
    "permissions",
];

/// One `agents/<name>.wcl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// The file stem.
    pub name: String,
    /// The display name: the name when the file has none.
    pub label: String,
    pub description: Option<String>,
    /// An icon name from the type icon set.
    pub icon: Option<String>,
    /// `false` keeps the file but runs no trigger and refuses new turns.
    pub enabled: bool,
    /// One of [`AGENT_PROVIDERS`]; `None` is the Settings > AI default provider.
    pub provider: Option<String>,
    /// `None` is the provider's default model in Settings > AI.
    pub model: Option<String>,
    /// One of [`AGENT_EFFORTS`]; `None` is the provider's default.
    pub effort: Option<String>,
    /// `None` is the shipped chat prompt.
    pub system_prompt: Option<String>,
    pub permissions: Permissions,
    /// In file order.
    pub triggers: Vec<Trigger>,
}

/// An agent's own permissions (agents G4). Nothing is inherited from Settings > AI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Permissions {
    /// The tools the agent may use; `None` is every tool the server offers.
    pub tools: Option<Vec<String>>,
    pub confirm: Confirm,
    pub web: Web,
    /// The note types the agent may read and change; `None` is every type.
    pub types: Option<Vec<String>>,
}

impl Permissions {
    /// Whether the agent may use tool `name`. The server still offers only what it can.
    pub fn allows_tool(&self, name: &str) -> bool {
        self.tools
            .as_ref()
            .is_none_or(|t| t.iter().any(|n| n == name))
    }

    /// Whether the agent may read and change notes of type `name`.
    pub fn allows_type(&self, name: &str) -> bool {
        self.types
            .as_ref()
            .is_none_or(|t| t.iter().any(|n| n == name))
    }

    fn is_default(&self) -> bool {
        *self == Permissions::default()
    }
}

/// Which kinds of change the agent proposes for the user to apply (`true`) rather than making
/// itself (`false`): the ai D22 switches. A switch left out of the file is `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirm {
    pub notes: bool,
    pub archive: bool,
    pub delete: bool,
    pub types: bool,
    pub downloads: bool,
    pub taxonomies: bool,
}

impl Default for Confirm {
    fn default() -> Confirm {
        Confirm {
            notes: true,
            archive: true,
            delete: true,
            types: true,
            downloads: true,
            taxonomies: true,
        }
    }
}

impl Confirm {
    /// The switches in [`CONFIRM_SWITCHES`] order.
    pub fn switches(&self) -> [(&'static str, bool); 6] {
        [
            ("notes", self.notes),
            ("archive", self.archive),
            ("delete", self.delete),
            ("types", self.types),
            ("downloads", self.downloads),
            ("taxonomies", self.taxonomies),
        ]
    }

    /// Set switch `name`, one of [`CONFIRM_SWITCHES`]. `false` for any other name.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        let slot = match name {
            "notes" => &mut self.notes,
            "archive" => &mut self.archive,
            "delete" => &mut self.delete,
            "types" => &mut self.types,
            "downloads" => &mut self.downloads,
            "taxonomies" => &mut self.taxonomies,
            _ => return false,
        };
        *slot = on;
        true
    }
}

/// Which internet tools the agent is given: the ai D28 switches. A switch left out of the file
/// is `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Web {
    pub search: bool,
    pub read: bool,
    pub download: bool,
}

impl Default for Web {
    fn default() -> Web {
        Web {
            search: true,
            read: true,
            download: true,
        }
    }
}

impl Web {
    /// The switches in [`WEB_SWITCHES`] order.
    pub fn switches(&self) -> [(&'static str, bool); 3] {
        [
            ("search", self.search),
            ("read", self.read),
            ("download", self.download),
        ]
    }

    /// Set switch `name`, one of [`WEB_SWITCHES`]. `false` for any other name.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        let slot = match name {
            "search" => &mut self.search,
            "read" => &mut self.read,
            "download" => &mut self.download,
            _ => return false,
        };
        *slot = on;
        true
    }
}

/// A `trigger <name> { ... }` block (agents G9, G10, G16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// The block label.
    pub name: String,
    /// The first user message of each run. A mention trigger's is optional: empty when the file
    /// has none, and the instruction from the note is then the whole message.
    pub prompt: String,
    pub when: When,
}

/// What starts a trigger's run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    Schedule(Schedule),
    NoteEvent(NoteEvent),
    Mention(Mention),
}

/// A mention trigger (agents G16): `mention = true`, with `type`. The agent answers a call,
/// Names an agent cannot take, because a route under `/api/agents/` already has them (agents G19:
/// `GET /api/agents/calls/{call}`).
pub const RESERVED_AGENT_NAMES: &[&str] = &["calls"];

/// The name of the mention trigger an agent with none of its own answers calls by (agents G16).
pub const IMPLICIT_MENTION: &str = "mention";

/// `@<name>` written in a note of one of `types`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mention {
    /// The note types whose notes may call the agent; empty is every type.
    pub types: Vec<String>,
}

impl Mention {
    /// Whether a note of `type_name` may call the agent by this trigger.
    pub fn allows(&self, type_name: &str) -> bool {
        self.types.is_empty() || self.types.iter().any(|t| t == type_name)
    }
}

/// A schedule trigger: `schedule`, with `timezone`, read as a task file's are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// The five-field cron expression as written.
    pub cron: String,
    /// The IANA time zone the schedule is read in; `"UTC"` when the file names none.
    pub timezone: String,
}

impl Schedule {
    /// The first time the schedule fires strictly after `after_ms`, in milliseconds since the
    /// Unix epoch. `None` when it never fires again, or for a schedule that does not parse.
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        let cron = parse_cron(&self.cron).ok()?;
        let tz = Tz::from_str(&self.timezone).ok()?;
        let at = tz.timestamp_millis_opt(after_ms).single()?;
        cron.find_next_occurrence(&at, false)
            .ok()
            .map(|t| t.timestamp_millis())
    }
}

/// A note event trigger: `on`, with `type` and `where`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEvent {
    /// Each of [`TRIGGER_OPS`] at most once, in file order.
    pub on: Vec<String>,
    /// The note types it fires for; empty is every type.
    pub types: Vec<String>,
    /// A condition in the query language, checked against the note after the change (before it,
    /// for a delete).
    pub condition: Option<String>,
}

impl NoteEvent {
    /// Whether a change `op` to a note of `type_name` passes `on` and `type`. The caller still
    /// checks [`NoteEvent::condition`].
    pub fn matches(&self, op: &str, type_name: &str) -> bool {
        self.on.iter().any(|o| o == op)
            && (self.types.is_empty() || self.types.iter().any(|t| t == type_name))
    }

    /// The parsed `where` condition.
    pub fn condition_expr(&self) -> Option<query_syntax::Expr> {
        self.condition
            .as_deref()
            .and_then(|c| query_syntax::parse_expr(c).ok())
    }
}

/// A five-field cron expression, as task schedules and agent triggers read it.
pub fn parse_cron(s: &str) -> Result<Cron, String> {
    if s.split_whitespace().count() != 5 {
        return Err(format!(
            "{s:?} is not a five-field cron expression: minute, hour, day of month, month, day of week"
        ));
    }
    Cron::new(s)
        .parse()
        .map_err(|e| format!("{s:?} is not a valid cron expression: {e}"))
}

impl Agent {
    /// The agent named `name` with nothing set: every default.
    pub fn new(name: &str) -> Agent {
        Agent {
            name: name.to_owned(),
            label: name.to_owned(),
            description: None,
            icon: None,
            enabled: true,
            provider: None,
            model: None,
            effort: None,
            system_prompt: None,
            permissions: Permissions::default(),
            triggers: Vec::new(),
        }
    }

    /// Whether this is the agent the vault ships, which cannot be deleted or disabled.
    pub fn is_built_in(&self) -> bool {
        self.name == ASSISTANT_AGENT
    }

    /// The trigger named `name`.
    pub fn trigger(&self, name: &str) -> Option<&Trigger> {
        self.triggers.iter().find(|t| t.name == name)
    }

    /// The mention trigger a note of `type_name` calls the agent by (agents G16): the first of
    /// its mention triggers that allows the type. An agent with no mention trigger answers a call
    /// from any type through an implicit one named `mention`, with no prompt and no type limit, so
    /// every agent can be called; a mention trigger only adds a prompt or limits the types.
    pub fn mention_trigger(&self, type_name: &str) -> Option<Trigger> {
        let mut any = false;
        for t in &self.triggers {
            if let When::Mention(m) = &t.when {
                any = true;
                if m.allows(type_name) {
                    return Some(t.clone());
                }
            }
        }
        (!any).then(|| Trigger {
            name: IMPLICIT_MENTION.to_owned(),
            prompt: String::new(),
            when: When::Mention(Mention { types: Vec::new() }),
        })
    }

    /// Read agent `name` from the text of `agents/<name>.wcl`. Every problem is reported at once.
    pub fn parse(name: &str, source: &str) -> Result<Agent, Invalid> {
        let file = format!("agents/{name}.wcl");
        let doc = Doc::parse(source, &file)?;
        Agent::from_doc(name, &doc).map_err(|issues| Invalid { file, issues })
    }

    pub fn from_doc(name: &str, doc: &Doc) -> Result<Agent, Vec<Issue>> {
        let mut issues = Vec::new();
        if !is_name(name) {
            issues.push(Issue::new(
                "name",
                format!(
                    "{name:?} is not a valid agent name: use [a-z][a-z0-9_]*, at most 64 characters"
                ),
            ));
        } else if RESERVED_AGENT_NAMES.contains(&name) {
            issues.push(Issue::new(
                "name",
                format!("{name:?} is reserved: /api/agents/{name} is a route of its own"),
            ));
        }
        let mut r = Reader {
            c: Check {
                issues: &mut issues,
                fields: None,
                template: false,
            },
        };
        r.known(doc, &FIELDS, "", "is not an agent field");
        for (field, _) in &doc.fields {
            if field == "permissions" {
                r.c.fail("permissions", "is a block: permissions { ... }");
            }
        }
        let mut agent = Agent::new(name);
        if let Some(label) = r.string(doc, "label", "label") {
            if label.trim().is_empty() {
                r.c.fail("label", "must not be empty");
            }
            agent.label = label;
        }
        agent.description = r.string(doc, "description", "description");
        agent.icon = r.form(doc, "icon", "icon", Form::Icon);
        agent.enabled = r.bool(doc, "enabled", "enabled").unwrap_or(true);
        agent.provider = r.form(doc, "provider", "provider", Form::OneOf(&AGENT_PROVIDERS));
        agent.model = r.string(doc, "model", "model");
        if agent.model.as_deref().is_some_and(|m| m.trim().is_empty()) {
            r.c.fail(
                "model",
                "must not be empty; leave it out for the default model",
            );
        }
        agent.effort = r.form(doc, "effort", "effort", Form::OneOf(&AGENT_EFFORTS));
        agent.system_prompt = r.prompt(doc, "system_prompt", "system_prompt", false);

        let mut permissions = 0;
        let mut seen = HashSet::new();
        for b in &doc.blocks {
            match b.kind.as_str() {
                "permissions" => {
                    permissions += 1;
                    if permissions > 1 {
                        r.c.fail("permissions", "appears more than once");
                        continue;
                    }
                    if !b.labels.is_empty() {
                        r.c.fail("permissions", "takes no label");
                    }
                    agent.permissions = r.permissions(&b.body);
                }
                "trigger" => {
                    if let Some(t) = r.trigger(b) {
                        if !seen.insert(t.name.clone()) {
                            r.c.fail(&format!("trigger {}", t.name), "appears more than once");
                        }
                        agent.triggers.push(t);
                    }
                }
                _ => r.c.fail(
                    &b.name(),
                    "is not an agent block: use permissions { ... } or trigger <name> { ... }",
                ),
            }
        }
        if issues.iter().any(|i| !i.is_warning()) {
            Err(issues)
        } else {
            Ok(agent)
        }
    }

    /// The rules between the agent and the vault's types: each type `permissions.types` or a
    /// trigger's `type` names must exist.
    pub fn check(&self, schema: &Schema) -> Vec<Issue> {
        let mut issues = Vec::new();
        for t in self.permissions.types.iter().flatten() {
            if schema.type_def(t).is_none() {
                issues.push(Issue::new(
                    "permissions.types",
                    format!("there is no type {t:?}"),
                ));
            }
        }
        for trigger in &self.triggers {
            let types = match &trigger.when {
                When::NoteEvent(e) => &e.types,
                When::Mention(m) => &m.types,
                When::Schedule(_) => continue,
            };
            for t in types {
                if schema.type_def(t).is_none() {
                    issues.push(Issue::new(
                        format!("trigger {}.type", trigger.name),
                        format!("there is no type {t:?}"),
                    ));
                }
            }
        }
        issues
    }

    /// The agent as the text of `agents/<name>.wcl`: every item that differs from its default,
    /// in the order the format lists them. The writer stores it in canonical form.
    pub fn to_wcl(&self) -> String {
        let mut out = String::new();
        if self.label != self.name {
            field(&mut out, "", "label", &quoted(&self.label));
        }
        let strings = [("description", &self.description), ("icon", &self.icon)];
        for (name, value) in strings {
            if let Some(v) = value {
                field(&mut out, "", name, &quoted(v));
            }
        }
        if !self.enabled {
            field(&mut out, "", "enabled", "false");
        }
        let strings = [
            ("provider", &self.provider),
            ("model", &self.model),
            ("effort", &self.effort),
            ("system_prompt", &self.system_prompt),
        ];
        for (name, value) in strings {
            if let Some(v) = value {
                field(&mut out, "", name, &quoted(v));
            }
        }
        let p = &self.permissions;
        if !p.is_default() {
            out.push_str("\npermissions {\n");
            if let Some(tools) = &p.tools {
                field(&mut out, "  ", "tools", &list(tools));
            }
            if let Some(types) = &p.types {
                field(&mut out, "  ", "types", &list(types));
            }
            let off: Vec<_> = p
                .confirm
                .switches()
                .into_iter()
                .filter(|(_, on)| !on)
                .collect();
            if !off.is_empty() {
                out.push_str("\n  confirm {\n");
                for (name, _) in off {
                    field(&mut out, "    ", name, "false");
                }
                out.push_str("  }\n");
            }
            let off: Vec<_> = p.web.switches().into_iter().filter(|(_, on)| !on).collect();
            if !off.is_empty() {
                out.push_str("\n  web {\n");
                for (name, _) in off {
                    field(&mut out, "    ", name, "false");
                }
                out.push_str("  }\n");
            }
            out.push_str("}\n");
        }
        for t in &self.triggers {
            out.push_str(&format!("\ntrigger {} {{\n", t.name));
            match &t.when {
                When::Schedule(s) => {
                    field(&mut out, "  ", "schedule", &quoted(&s.cron));
                    if s.timezone != "UTC" {
                        field(&mut out, "  ", "timezone", &quoted(&s.timezone));
                    }
                }
                When::NoteEvent(e) => {
                    field(&mut out, "  ", "on", &list(&e.on));
                    match e.types.as_slice() {
                        [] => {}
                        [one] => field(&mut out, "  ", "type", &quoted(one)),
                        many => field(&mut out, "  ", "type", &list(many)),
                    }
                    if let Some(c) = &e.condition {
                        field(&mut out, "  ", "where", &quoted(c));
                    }
                }
                When::Mention(m) => {
                    field(&mut out, "  ", "mention", "true");
                    match m.types.as_slice() {
                        [] => {}
                        [one] => field(&mut out, "  ", "type", &quoted(one)),
                        many => field(&mut out, "  ", "type", &list(many)),
                    }
                }
            }
            if !(matches!(t.when, When::Mention(_)) && t.prompt.is_empty()) {
                field(&mut out, "  ", "prompt", &quoted(&t.prompt));
            }
            out.push_str("}\n");
        }
        out.trim_start_matches('\n').to_owned()
    }
}

fn field(out: &mut String, indent: &str, name: &str, value: &str) {
    out.push_str(indent);
    out.push_str(name);
    out.push_str(" = ");
    out.push_str(value);
    out.push('\n');
}

/// `s` as a WCL string literal.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|s| quoted(s)).collect();
    format!("[{}]", items.join(", "))
}

/// The trigger fields, by kind.
const TRIGGER_FIELDS: [&str; 7] = [
    "schedule", "timezone", "on", "mention", "type", "where", "prompt",
];

struct Reader<'a> {
    c: Check<'a>,
}

impl Reader<'_> {
    /// Report a field outside `known` and a repeated field.
    fn known(&mut self, doc: &Doc, known: &[&str], prefix: &str, unknown: &str) {
        let mut seen = HashSet::new();
        for (name, _) in &doc.fields {
            let at = format!("{prefix}{name}");
            if !seen.insert(name.as_str()) {
                self.c.fail(&at, "appears more than once");
            } else if !known.contains(&name.as_str()) {
                self.c.fail(&at, unknown);
            }
        }
    }

    fn form(&mut self, doc: &Doc, name: &str, at: &str, form: Form) -> Option<String> {
        let v = doc.field(name)?;
        let before = self.c.errors();
        self.c.form(v, form, at);
        (self.c.errors() == before)
            .then(|| v.as_str().map(str::to_owned))
            .flatten()
    }

    fn string(&mut self, doc: &Doc, name: &str, at: &str) -> Option<String> {
        self.form(doc, name, at, Form::Str)
    }

    fn bool(&mut self, doc: &Doc, name: &str, at: &str) -> Option<bool> {
        match doc.field(name)? {
            Value::Bool(b) => Some(*b),
            v => {
                self.c
                    .fail(at, format!("must be true or false, not {}", v.describe()));
                None
            }
        }
    }

    /// A prompt: a string of at most [`AGENT_PROMPT_MAX`] characters, not blank.
    fn prompt(&mut self, doc: &Doc, name: &str, at: &str, required: bool) -> Option<String> {
        if required && doc.field(name).is_none() {
            self.c.fail(at, "is required");
            return None;
        }
        let p = self.string(doc, name, at)?;
        if p.trim().is_empty() {
            self.c.fail(at, "must not be empty");
        } else if p.chars().count() > AGENT_PROMPT_MAX {
            self.c
                .fail(at, format!("is longer than {AGENT_PROMPT_MAX} characters"));
        }
        Some(p)
    }

    /// A list of names, each matching [`is_name`] when `names`, each at most once.
    fn names(&mut self, doc: &Doc, field: &str, at: &str, what: &str) -> Option<Vec<String>> {
        let v = doc.field(field)?;
        let before = self.c.errors();
        let items: Vec<String> = match v {
            Value::Str(s) => vec![s.clone()],
            Value::List(_) => {
                self.c.form(v, Form::StrList, at);
                match v {
                    Value::List(l) => l
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                    _ => Vec::new(),
                }
            }
            v => {
                self.c.fail(
                    at,
                    format!("must be a list of strings, not {}", v.describe()),
                );
                Vec::new()
            }
        };
        let mut seen = HashSet::new();
        for i in &items {
            if !is_name(i) {
                self.c.fail(at, format!("{i:?} is not a {what} name"));
            } else if !seen.insert(i.as_str()) {
                self.c.fail(at, format!("{i:?} appears more than once"));
            }
        }
        (self.c.errors() == before).then_some(items)
    }

    fn permissions(&mut self, d: &Doc) -> Permissions {
        self.known(
            d,
            &["tools", "types"],
            "permissions.",
            "is not a permission: use tools, types, confirm { ... } or web { ... }",
        );
        let mut p = Permissions {
            tools: self.names(d, "tools", "permissions.tools", "tool"),
            types: self.names(d, "types", "permissions.types", "type"),
            ..Permissions::default()
        };
        let mut seen = HashSet::new();
        for b in &d.blocks {
            let at = format!("permissions.{}", b.kind);
            let switches: &[&str] = match b.kind.as_str() {
                "confirm" => &CONFIRM_SWITCHES,
                "web" => &WEB_SWITCHES,
                _ => {
                    self.c.fail(
                        &format!("permissions > {}", b.name()),
                        "is not a permissions block: use confirm { ... } or web { ... }",
                    );
                    continue;
                }
            };
            if !seen.insert(b.kind.as_str()) {
                self.c.fail(&at, "appears more than once");
                continue;
            }
            if !b.labels.is_empty() {
                self.c.fail(&at, "takes no label");
            }
            self.c.only_blocks(&b.body, &[], &at);
            self.known(
                &b.body,
                switches,
                &format!("{at}."),
                &format!("is not a switch: use {}", switches.join(", ")),
            );
            for (name, value) in &b.body.fields {
                let Some(on) = (match value {
                    Value::Bool(on) => Some(*on),
                    v => {
                        self.c.fail(
                            &format!("{at}.{name}"),
                            format!("must be true or false, not {}", v.describe()),
                        );
                        None
                    }
                }) else {
                    continue;
                };
                if b.kind == "confirm" {
                    p.confirm.set(name, on);
                } else {
                    p.web.set(name, on);
                }
            }
        }
        p
    }

    fn trigger(&mut self, b: &DocBlock) -> Option<Trigger> {
        let name = b.label().unwrap_or("").to_owned();
        let at = format!("trigger {name}");
        if b.labels.len() != 1 || !is_name(&name) {
            self.c.fail(
                &at,
                "the label must be one trigger name: [a-z][a-z0-9_]*, at most 64 characters",
            );
        }
        let d = &b.body;
        self.c.only_blocks(d, &[], &at);
        self.known(
            d,
            &TRIGGER_FIELDS,
            &format!("{at}."),
            "is not a trigger field",
        );
        let schedule = d.field("schedule").is_some();
        let event = d.field("on").is_some();
        if d.field("mention").is_some() {
            return self.mention_trigger(d, name, &at, schedule || event);
        }
        let prompt = self.prompt(d, "prompt", &format!("{at}.prompt"), true);
        let when = match (schedule, event) {
            (true, true) => {
                self.c.fail(
                    &at,
                    "has both schedule and on: a trigger is one or the other",
                );
                None
            }
            (false, false) => {
                self.c.fail(
                    &at,
                    "needs schedule, a cron expression, on, a list of note changes, or mention = true",
                );
                None
            }
            (true, false) => {
                for f in ["type", "where"] {
                    if d.field(f).is_some() {
                        self.c.fail(
                            &format!("{at}.{f}"),
                            "belongs to a note event trigger, which has on in place of schedule",
                        );
                    }
                }
                self.schedule(d, &at).map(When::Schedule)
            }
            (false, true) => {
                if d.field("timezone").is_some() {
                    self.c
                        .fail(&format!("{at}.timezone"), "belongs to a schedule trigger");
                }
                self.note_event(d, &at).map(When::NoteEvent)
            }
        };
        Some(Trigger {
            name,
            prompt: prompt?,
            when: when?,
        })
    }

    /// A mention trigger (agents G16): `mention = true`, an optional `type` and an optional
    /// `prompt`, and nothing of the other kinds.
    fn mention_trigger(
        &mut self,
        d: &Doc,
        name: String,
        at: &str,
        other_kind: bool,
    ) -> Option<Trigger> {
        let before = self.c.errors();
        if other_kind {
            self.c.fail(
                at,
                "has mention with schedule or on: a trigger is one kind only",
            );
        }
        for f in ["timezone", "where"] {
            if d.field(f).is_some() {
                self.c.fail(
                    &format!("{at}.{f}"),
                    "does not belong to a mention trigger, which takes type and prompt",
                );
            }
        }
        match d.field("mention") {
            Some(Value::Bool(true)) | None => {}
            Some(Value::Bool(false)) => self.c.fail(
                &format!("{at}.mention"),
                "must be true; leave the trigger out to answer no calls",
            ),
            Some(v) => self.c.fail(
                &format!("{at}.mention"),
                format!("must be true, not {}", v.describe()),
            ),
        }
        let prompt = if d.field("prompt").is_some() {
            self.prompt(d, "prompt", &format!("{at}.prompt"), false)
        } else {
            Some(String::new())
        };
        let types = self
            .names(d, "type", &format!("{at}.type"), "type")
            .unwrap_or_default();
        (self.c.errors() == before).then_some(Trigger {
            name,
            prompt: prompt?,
            when: When::Mention(Mention { types }),
        })
    }

    fn schedule(&mut self, d: &Doc, at: &str) -> Option<Schedule> {
        let cron = self.string(d, "schedule", &format!("{at}.schedule"))?;
        let mut ok = true;
        if let Err(e) = parse_cron(&cron) {
            self.c.fail(&format!("{at}.schedule"), e);
            ok = false;
        }
        let timezone = match self.string(d, "timezone", &format!("{at}.timezone")) {
            None if d.field("timezone").is_some() => return None,
            None => "UTC".to_owned(),
            Some(z) => {
                if Tz::from_str(&z).is_err() {
                    self.c.fail(
                        &format!("{at}.timezone"),
                        format!("{z:?} is not an IANA time zone"),
                    );
                    ok = false;
                }
                z
            }
        };
        ok.then_some(Schedule { cron, timezone })
    }

    fn note_event(&mut self, d: &Doc, at: &str) -> Option<NoteEvent> {
        let on_at = format!("{at}.on");
        let before = self.c.errors();
        let on: Vec<String> = match d.field("on") {
            Some(Value::List(l)) if !l.is_empty() => {
                let mut out = Vec::new();
                for v in l {
                    match v.as_str() {
                        Some(op) if TRIGGER_OPS.contains(&op) => {
                            if out.iter().any(|o| o == op) {
                                self.c
                                    .fail(&on_at, format!("{op:?} appears more than once"));
                            }
                            out.push(op.to_owned());
                        }
                        Some(op) => self.c.fail(
                            &on_at,
                            format!(
                                "{op:?} is not a note change: use {}",
                                TRIGGER_OPS.join(", ")
                            ),
                        ),
                        None => self.c.fail(
                            &on_at,
                            format!("must hold strings only, not {}", v.describe()),
                        ),
                    }
                }
                out
            }
            Some(Value::List(_)) => {
                self.c.fail(&on_at, "must name at least one note change");
                Vec::new()
            }
            Some(v) => {
                self.c.fail(
                    &on_at,
                    format!(
                        "must be a list of note changes such as [\"create\"], not {}",
                        v.describe()
                    ),
                );
                Vec::new()
            }
            None => Vec::new(),
        };
        let types = self
            .names(d, "type", &format!("{at}.type"), "type")
            .unwrap_or_default();
        let where_at = format!("{at}.where");
        let condition = self.string(d, "where", &where_at);
        if let Some(c) = &condition {
            if c.trim().is_empty() {
                self.c.fail(
                    &where_at,
                    "must not be empty; leave it out to match every note",
                );
            } else if let Err(e) = query_syntax::parse_expr(c) {
                self.c
                    .fail(&where_at, format!("is not a query condition: {e}"));
            }
        }
        (self.c.errors() == before).then_some(NoteEvent {
            on,
            types,
            condition,
        })
    }
}
