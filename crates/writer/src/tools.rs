//! Script tool files, `tools/<name>.wcl` (script-tools T2): a script function an agent calls as a
//! tool. A tool file is a definition, written in canonical form, synced like scripts.
//!
//! [`Writer::put_tool`] writes the tool file and, when given, the script it names in one commit.
//! [`Writer::delete_tool`] deletes the tool file, and its script too when nothing else (a handler,
//! a task, a device file or another tool) names it.

use schema::{Doc, Invalid, Issue, ScriptTool, Value};
use store::Change;

use crate::defs::script_path;
use crate::{Error, Result, ScriptRole, Writer};

/// The folder tool files live in.
pub const TOOLS_DIR: &str = "tools";

/// The vault path of tool `name`.
pub fn tool_path(name: &str) -> String {
    format!("{TOOLS_DIR}/{name}.wcl")
}

/// A tool as it was read for one run: its file, its script's source, and the vault generation
/// both were read at (script-tools T6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSnapshot {
    pub wcl: String,
    /// `None` when the script the tool names does not exist, or the tool file does not parse.
    pub script: Option<String>,
    pub generation: u64,
}

fn check_tool_name(name: &str) -> Result<()> {
    if schema::is_name(name) {
        Ok(())
    } else {
        Err(Error::BadRequest(format!(
            "{name:?} is not a valid tool name: use [a-z][a-z0-9_]*, at most 64 characters"
        )))
    }
}

fn missing_tool(name: &str) -> Error {
    Error::Missing(format!("there is no tool {name:?}"))
}

/// One way a script runs, and what that run may reach ([`Writer::script_runs_as`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptUse {
    /// `handler` (a type's), `task`, `device` or `tool`.
    pub kind: &'static str,
    /// The type, task, device platform or tool.
    pub name: String,
    /// Its vault file.
    pub file: String,
    /// A handler's event (`before_update`, ...), or a device file's `handler <event>` or
    /// `command <name>`.
    pub event: Option<String>,
    /// Hosts its `http` module may reach; `"*"` is any host.
    pub allow_domains: Vec<String>,
    /// Secrets it may read and write.
    pub secrets: Vec<String>,
    /// A task's `fs` directories.
    pub read_paths: Vec<String>,
    pub write_paths: Vec<String>,
    /// A task's schedule as written, its time zone, and whether it is paused.
    pub schedule: Option<String>,
    pub timezone: Option<String>,
    pub paused: bool,
}

impl ScriptUse {
    fn new(kind: &'static str, name: &str, file: String) -> ScriptUse {
        ScriptUse {
            kind,
            name: name.to_owned(),
            file,
            ..ScriptUse::default()
        }
    }
}

/// The strings of list field `field`.
fn strings(doc: &Doc, field: &str) -> Vec<String> {
    match doc.field(field) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// The script a tool file names, read without checking the rest of it.
fn script_named(text: &str, file: &str) -> Option<String> {
    let doc = Doc::parse(text, file).ok()?;
    doc.field("script")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Writer {
    /// The `(name, text)` of every `tools/<name>.wcl` with a valid name, sorted by name.
    pub fn tool_files(&self) -> Result<Vec<(String, String)>> {
        let mut files: Vec<_> = self
            .definitions(TOOLS_DIR)?
            .into_iter()
            .filter(|(n, _)| schema::is_name(n))
            .collect();
        files.sort();
        Ok(files)
    }

    /// Every tool file, parsed. A file that does not parse is listed with its problems.
    pub fn tools(&self) -> Result<Vec<(String, std::result::Result<ScriptTool, Invalid>)>> {
        Ok(self
            .tool_files()?
            .into_iter()
            .map(|(name, text)| {
                let tool = ScriptTool::parse(&name, &text);
                (name, tool)
            })
            .collect())
    }

    /// The text of `tools/<name>.wcl`.
    pub fn read_tool(&self, name: &str) -> Result<String> {
        check_tool_name(name)?;
        match self.vault.read_text(&tool_path(name)) {
            Err(store::Error::NotFound(_)) => Err(missing_tool(name)),
            other => Ok(other?),
        }
    }

    /// Tool `name` and the source of its script as they are now, with the generation they were
    /// read at, read under the writer's lock so no commit falls between them.
    pub fn tool_snapshot(&self, name: &str) -> Result<ToolSnapshot> {
        let _inner = self.lock();
        let wcl = self.read_tool(name)?;
        let script = script_named(&wcl, &tool_path(name))
            .filter(|s| schema::is_name(s))
            .and_then(|s| self.vault.read_text(&script_path(&s)).ok());
        Ok(ToolSnapshot {
            wcl,
            script,
            generation: self.vault.generation()?,
        })
    }

    /// Check `source` as `tools/<name>.wcl` without writing it: the file alone, and by `check`,
    /// which the caller uses for what only it knows, such as the built-in tool names. Returns the
    /// canonical text and the tool.
    pub fn check_tool(
        &self,
        name: &str,
        source: &str,
        check: impl FnOnce(&ScriptTool) -> Vec<Issue>,
    ) -> Result<(String, ScriptTool)> {
        check_tool_name(name)?;
        let path = tool_path(name);
        let text = store::canonical(source, &path)?;
        let tool = ScriptTool::parse(name, &text).map_err(Error::Invalid)?;
        let issues = check(&tool);
        if issues.iter().any(|i| !i.is_warning()) {
            return Err(Error::Invalid(Invalid { file: path, issues }));
        }
        Ok((text, tool))
    }

    /// Create or replace `tools/<name>.wcl` in canonical form, checked as
    /// [`Writer::check_tool`] checks it, and with `script` the source of the script it names, in
    /// one commit. A script that does not exist or does not compile is not refused: the tool
    /// routes report it as a problem, and the tool is not offered until it is fixed.
    pub fn put_tool(
        &self,
        name: &str,
        source: &str,
        script: Option<&str>,
        check: impl FnOnce(&ScriptTool) -> Vec<Issue>,
    ) -> Result<(String, ScriptTool)> {
        let (text, tool, changes) = self.put_tool_changes(name, source, script, check)?;
        self.commit_checked(|_| Ok::<_, Error>(changes))?;
        Ok((text, tool))
    }

    /// The changes [`Writer::put_tool`] writes, for [`Writer::commit_checked`]: the tool file, and
    /// `script` when given. Also the canonical text and the tool.
    pub fn put_tool_changes(
        &self,
        name: &str,
        source: &str,
        script: Option<&str>,
        check: impl FnOnce(&ScriptTool) -> Vec<Issue>,
    ) -> Result<(String, ScriptTool, Vec<Change>)> {
        let (text, tool) = self.check_tool(name, source, check)?;
        let mut changes = Vec::new();
        if let Some(s) = script {
            changes.push(Change::write(script_path(&tool.script), s.as_bytes()));
        }
        changes.push(Change::write(tool_path(name), text.as_bytes()));
        Ok((text, tool, changes))
    }

    /// Delete `tools/<name>.wcl`, and the script it names when nothing else names it: no type's
    /// handler, task, device file or other tool. Returns the script deleted with it, if any.
    pub fn delete_tool(&self, name: &str) -> Result<Option<String>> {
        let mut deleted = None;
        self.commit_checked(|w| -> Result<Vec<Change>> {
            let (changes, script) = w.delete_tool_changes(name)?;
            deleted = script;
            Ok(changes)
        })?;
        Ok(deleted)
    }

    /// The changes [`Writer::delete_tool`] makes, for [`Writer::commit_checked`], and the script
    /// they delete with the tool, if any.
    pub fn delete_tool_changes(&self, name: &str) -> Result<(Vec<Change>, Option<String>)> {
        let text = self.read_tool(name)?;
        let schema = self.schema();
        let script = self.tool_script_to_delete(&schema, name, &text)?;
        let mut changes = vec![Change::delete(tool_path(name))];
        if let Some(s) = &script {
            changes.push(Change::delete(script_path(s)));
        }
        Ok((changes, script))
    }

    /// The script deleting tool `name` (whose file is `text`) also deletes: the one it names, when
    /// it exists and nothing else names it.
    pub fn tool_script_to_delete(
        &self,
        schema: &schema::Schema,
        name: &str,
        text: &str,
    ) -> Result<Option<String>> {
        let Some(script) = script_named(text, &tool_path(name)).filter(|s| schema::is_name(s))
        else {
            return Ok(None);
        };
        if self.vault.read_text(&script_path(&script)).is_err() {
            return Ok(None);
        }
        let users = self.script_users(schema, &script, Some(name))?;
        Ok(users.is_empty().then_some(script))
    }

    /// The script deleting tool `name` would also delete, as [`Writer::delete_tool`] decides it.
    pub fn tool_delete_takes_script(&self, name: &str) -> Result<Option<String>> {
        let text = self.read_tool(name)?;
        let schema = self.schema();
        self.tool_script_to_delete(&schema, name, &text)
    }

    /// The vault files that name script `name`: type handlers, tasks, device files and tools,
    /// leaving out tool `except`.
    pub(crate) fn script_users(
        &self,
        schema: &schema::Schema,
        name: &str,
        except: Option<&str>,
    ) -> Result<Vec<String>> {
        let mut by: Vec<String> = schema
            .types()
            .flat_map(|t| {
                t.handlers
                    .iter()
                    .filter(|h| h.script == name)
                    .map(|h| format!("{} (handler {:?})", crate::type_path(&t.name), h.event))
            })
            .collect();
        for (task, text) in self.definitions("tasks")? {
            let file = crate::defs::task_path(&task);
            if let Ok(doc) = Doc::parse(&text, &file)
                && doc.field("script").and_then(Value::as_str) == Some(name)
            {
                by.push(file);
            }
        }
        by.extend(self.device_files_naming(name)?);
        for (tool, text) in self.tool_files()? {
            if Some(tool.as_str()) == except {
                continue;
            }
            let file = tool_path(&tool);
            if script_named(&text, &file).as_deref() == Some(name) {
                by.push(file);
            }
        }
        Ok(by)
    }

    /// Every way script `name` runs, with the grants each gives it (TV-70, the "Runs as" list of
    /// the script and tool proposal cards): type handlers, tasks, device files and tools, in that
    /// order, leaving out tool `except`. The files [`Writer::script_users`] names, read in full.
    /// A task or tool file that does not parse is listed with no grants.
    pub fn script_runs_as(&self, name: &str, except: Option<&str>) -> Result<Vec<ScriptUse>> {
        let schema = self.schema();
        let mut uses: Vec<ScriptUse> = schema
            .types()
            .flat_map(|t| {
                t.handlers
                    .iter()
                    .filter(|h| h.script == name)
                    .map(|h| ScriptUse {
                        allow_domains: h.allow_domains.clone(),
                        secrets: h.secrets.clone(),
                        event: Some(h.event.clone()),
                        ..ScriptUse::new("handler", &t.name, crate::type_path(&t.name))
                    })
            })
            .collect();
        for (task, text) in self.task_files()? {
            let file = crate::defs::task_path(&task);
            let Ok(doc) = Doc::parse(&text, &file) else {
                continue;
            };
            if doc.field("script").and_then(Value::as_str) != Some(name) {
                continue;
            }
            let field = |f: &str| doc.field(f).and_then(Value::as_str).map(str::to_owned);
            uses.push(ScriptUse {
                allow_domains: strings(&doc, "allow_domains"),
                secrets: strings(&doc, "secrets"),
                read_paths: strings(&doc, "read_paths"),
                write_paths: strings(&doc, "write_paths"),
                schedule: field("schedule"),
                timezone: field("timezone"),
                paused: matches!(doc.field("paused"), Some(Value::Bool(true))),
                ..ScriptUse::new("task", &task, file)
            });
        }
        for (platform, text) in self.device_files()? {
            let Ok(file) = schema::DeviceFile::parse(&platform, &text) else {
                continue;
            };
            let path = crate::device_file_path(&platform);
            let hooks = file
                .handlers
                .iter()
                .map(|h| (h, "handler"))
                .chain(file.commands.iter().map(|c| (c, "command")));
            for (h, what) in hooks.filter(|(h, _)| h.script == name) {
                uses.push(ScriptUse {
                    event: Some(format!("{what} {}", h.name)),
                    ..ScriptUse::new("device", &platform, path.clone())
                });
            }
        }
        for (tool, text) in self.tool_files()? {
            let file = tool_path(&tool);
            if Some(tool.as_str()) == except || script_named(&text, &file).as_deref() != Some(name)
            {
                continue;
            }
            let (allow_domains, secrets) = ScriptTool::parse(&tool, &text)
                .map(|t| (t.allow_domains, t.secrets))
                .unwrap_or_default();
            uses.push(ScriptUse {
                allow_domains,
                secrets,
                ..ScriptUse::new("tool", &tool, file)
            });
        }
        Ok(uses)
    }

    /// Whether some tool file names script `name`.
    pub(crate) fn tools_naming(&self, name: &str) -> bool {
        self.tool_files()
            .unwrap_or_default()
            .iter()
            .any(|(tool, text)| script_named(text, &tool_path(tool)).as_deref() == Some(name))
    }

    /// Why `source` would not compile as script `name` run as a tool or a task, which reach the
    /// same modules (script-tools T2).
    pub fn check_tool_script(&self, name: &str, source: &str) -> std::result::Result<(), String> {
        self.hooks.check_script(ScriptRole::Task, name, source)
    }
}
