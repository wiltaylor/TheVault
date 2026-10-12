//! Definitions other than whole types: the templates inside a type file (D14), taxonomies with
//! their hierarchy edits (D7), scripts, and the canvases inside notes (D15).
//!
//! A change to a definition file reloads the schema from the files as they would be after it, and
//! is refused when the definitions would no longer check. A taxonomy edit that renames or moves
//! terms rewrites every note holding those terms in the same commit; one that drops a term a note
//! holds is refused.

use std::collections::BTreeMap;
use std::sync::MutexGuard;

use schema::{Doc, DocBlock, FieldKind, Invalid, Issue, Schema, Taxonomy, Value};
use state::Author;
use store::Change;
use wcl_lang::ast::{Expr, Item, Source};

use crate::{
    Error, Event, Inner, Input, NoteChange, Result, Saved, ScriptRole, Writer, created_of, note,
    note_path, pairs, time, type_path,
};

/// Why a script would not compile in one of the ways the vault runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptProblem {
    pub role: ScriptRole,
    pub message: String,
}

/// The vault path of taxonomy `name`.
pub fn taxonomy_path(name: &str) -> String {
    format!("taxonomies/{name}.wcl")
}

/// The vault path of task `name`.
pub fn task_path(name: &str) -> String {
    format!("tasks/{name}.wcl")
}

/// The vault path of script `name`.
pub fn script_path(name: &str) -> String {
    format!("scripts/{name}.ws")
}

fn first_label(labels: &[Expr]) -> Option<&str> {
    match labels.first()? {
        Expr::Utf8(s) | Expr::Identifier(s, _) => Some(s),
        _ => None,
    }
}

fn is_template(item: &Item, name: &str) -> bool {
    matches!(item, Item::Block(b) if b.kind == "template" && first_label(&b.labels) == Some(name))
}

fn check_name(what: &str, name: &str) -> Result<()> {
    if schema::is_name(name) {
        Ok(())
    } else {
        Err(Error::BadRequest(format!(
            "{name:?} is not a valid {what} name: use [a-z][a-z0-9_]*, at most 64 characters"
        )))
    }
}

/// `path` renamed by the longest entry of `renames` that is the path itself or one of its
/// ancestors.
fn rename_term(path: &str, renames: &BTreeMap<String, String>) -> Option<String> {
    renames
        .iter()
        .filter(|(from, _)| {
            path == from.as_str()
                || path
                    .strip_prefix(from.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|(from, _)| from.len())
        .map(|(from, to)| format!("{to}{}", &path[from.len()..]))
}

impl Writer {
    /// Replace or delete definition files and reload the schema from them, writing `notes` in the
    /// same commit. Refused when the definitions would not check.
    fn commit_definitions(
        &self,
        mut inner: MutexGuard<'_, Inner>,
        files: Vec<(String, Option<String>)>,
        notes: Vec<NoteChange>,
        author: &Author,
    ) -> Result<Vec<Saved>> {
        let mut types = self.definitions("types")?;
        let mut taxonomies = self.definitions("taxonomies")?;
        for (path, text) in &files {
            let (list, stem) = if let Some(stem) = path.strip_prefix("types/") {
                (&mut types, stem)
            } else if let Some(stem) = path.strip_prefix("taxonomies/") {
                (&mut taxonomies, stem)
            } else {
                continue;
            };
            let stem = stem.trim_end_matches(".wcl");
            list.retain(|(n, _)| n != stem);
            if let Some(text) = text {
                list.push((stem.to_owned(), text.clone()));
            }
        }
        let schema = Schema::from_sources(&pairs(&types), &pairs(&taxonomies))
            .map_err(Error::InvalidType)?;
        let changes: Vec<Change> = files
            .into_iter()
            .map(|(path, text)| match text {
                Some(text) => Change::write(path, text),
                None => Change::delete(path),
            })
            .collect();
        if notes.is_empty() {
            self.vault.commit(changes)?;
            inner.index.set_schema(&schema)?;
            inner.schema = schema;
            return Ok(Vec::new());
        }
        inner.index.set_schema(&schema)?;
        let old = std::mem::replace(&mut inner.schema, schema);
        let result = self.apply_with(inner, notes, &|| Ok(()), author, changes, false);
        if result.is_err() {
            let mut inner = self.lock();
            inner.index.set_schema(&old)?;
            inner.schema = old;
        }
        result
    }

    // ---- templates ------------------------------------------------------------------------

    /// The type file of `type_name`, parsed.
    fn type_ast(&self, inner: &Inner, type_name: &str) -> Result<(String, Source)> {
        if inner.schema.type_def(type_name).is_none() {
            return Err(Error::UnknownType(type_name.to_owned()));
        }
        let path = type_path(type_name);
        let text = self.vault.read_text(&path)?;
        Ok((path.clone(), store::parse_data_only(&text, &path)?))
    }

    /// The canonical WCL of template `name` of `type_name`: the `template <name> { ... }` block as
    /// it stands in the type file.
    pub fn template(&self, type_name: &str, name: &str) -> Result<String> {
        let inner = self.lock();
        let (_, ast) = self.type_ast(&inner, type_name)?;
        let item = ast
            .items
            .into_iter()
            .find(|i| is_template(i, name))
            .ok_or_else(|| missing_template(type_name, name))?;
        Ok(store::to_canonical(&Source {
            items: vec![item],
            trailing_trivia: Vec::new(),
        }))
    }

    /// Create or replace template `name` of `type_name` from the WCL of its `template` block.
    /// Templates change no note, so this works on a type that has notes. Returns the block as
    /// stored.
    pub fn put_template(&self, type_name: &str, name: &str, source: &str) -> Result<String> {
        let _w = self.write_lock();
        let inner = self.lock();
        if !schema::is_ident(name) {
            return Err(Error::BadRequest(format!(
                "{name:?} is not a valid template name"
            )));
        }
        let (path, mut ast) = self.type_ast(&inner, type_name)?;
        let submitted = store::parse_data_only(source, &format!("{path} template {name}"))?;
        let item = match submitted.items.as_slice() {
            [item @ Item::Block(b)] if b.kind == "template" => {
                if first_label(&b.labels) != Some(name) {
                    return Err(Error::BadRequest(format!(
                        "send one `template {name} {{ ... }}` block: its label must be {name:?}"
                    )));
                }
                item.clone()
            }
            _ => {
                return Err(Error::BadRequest(format!(
                    "send exactly one `template {name} {{ ... }}` block"
                )));
            }
        };
        match ast.items.iter().position(|i| is_template(i, name)) {
            Some(n) => ast.items[n] = item,
            None => {
                let after = ast
                    .items
                    .iter()
                    .rposition(|i| matches!(i, Item::Block(b) if b.kind == "template"))
                    .map_or(ast.items.len(), |n| n + 1);
                ast.items.insert(after, item);
            }
        }
        let text = store::to_canonical(&ast);
        let author = Author::Session(crate::ADOPTED_BY.to_owned());
        self.commit_definitions(inner, vec![(path, Some(text))], Vec::new(), &author)?;
        self.template(type_name, name)
    }

    /// Delete template `name` of `type_name`. The `default` template cannot be deleted.
    pub fn delete_template(&self, type_name: &str, name: &str) -> Result<()> {
        let _w = self.write_lock();
        let inner = self.lock();
        if name == "default" {
            return Err(Error::BadRequest(
                "every type needs a `default` template, so it cannot be deleted".to_owned(),
            ));
        }
        let (path, mut ast) = self.type_ast(&inner, type_name)?;
        let n = ast
            .items
            .iter()
            .position(|i| is_template(i, name))
            .ok_or_else(|| missing_template(type_name, name))?;
        ast.items.remove(n);
        let text = store::to_canonical(&ast);
        let author = Author::Session(crate::ADOPTED_BY.to_owned());
        self.commit_definitions(inner, vec![(path, Some(text))], Vec::new(), &author)?;
        Ok(())
    }

    // ---- taxonomies -----------------------------------------------------------------------

    /// The canonical text of `taxonomies/<name>.wcl`.
    pub fn read_taxonomy(&self, name: &str) -> Result<String> {
        let inner = self.lock();
        if inner.schema.taxonomy(name).is_none() {
            return Err(missing_taxonomy(name));
        }
        Ok(self.vault.read_text(&taxonomy_path(name))?)
    }

    /// Create or replace taxonomy `name`. `renames` maps old term paths to new ones: a renamed or
    /// moved term takes its children with it, and every note holding a renamed term, or a term
    /// below it, is rewritten in the same commit. A change that drops a term some note holds is
    /// refused. Returns the stored text and the notes rewritten.
    pub fn put_taxonomy(
        &self,
        name: &str,
        source: &str,
        renames: &BTreeMap<String, String>,
        author: &Author,
    ) -> Result<(String, Vec<Saved>)> {
        let _w = self.write_lock();
        let inner = self.lock();
        check_name("taxonomy", name)?;
        let path = taxonomy_path(name);
        let text = store::canonical(source, &path)?;
        let new = Taxonomy::parse(name, &text)
            .map_err(|inv| Error::InvalidType(schema::SchemaError(vec![inv])))?;
        let old = inner.schema.taxonomy(name).cloned();
        for (from, to) in renames {
            if !old.as_ref().is_some_and(|t| t.contains(from)) {
                return Err(Error::BadRequest(format!(
                    "rename {from:?}: taxonomy {name:?} has no such term"
                )));
            }
            if !new.contains(to) {
                return Err(Error::BadRequest(format!(
                    "rename {from:?} to {to:?}: the new taxonomy has no term {to:?}"
                )));
            }
        }

        let mut changes = Vec::new();
        let mut stranded = Vec::new();
        let now = time::utc(self.clock.now_ms());
        for ty in inner.schema.types() {
            let fields: Vec<&str> = ty
                .fields
                .iter()
                .filter(|f| matches!(&f.kind, FieldKind::Taxonomy { taxonomy } if taxonomy == name))
                .map(|f| f.name.as_str())
                .collect();
            if fields.is_empty() {
                continue;
            }
            for id in self.ids_of_type(&inner, &ty.name)? {
                let file = note_path(&ty.name, &id);
                let mut doc = Input::Wcl(self.vault.read_text(&file)?).parse(&file)?;
                let mut changed = false;
                for (field, value) in doc.fields.iter_mut() {
                    if !fields.contains(&field.as_str()) {
                        continue;
                    }
                    let terms: Vec<&mut Value> = match value {
                        Value::List(l) => l.iter_mut().collect(),
                        v => vec![v],
                    };
                    for term in terms {
                        let Value::Str(s) = term else { continue };
                        if let Some(to) = rename_term(s, renames) {
                            *s = to;
                            changed = true;
                        }
                        if !new.contains(s) {
                            stranded.push(format!("{file} ({field} = {s:?})"));
                        }
                    }
                }
                if changed {
                    let created = created_of(&doc).unwrap_or_else(|| now.clone());
                    changes.push(NoteChange {
                        id,
                        type_name: ty.name.clone(),
                        event: Event::Update,
                        doc: Some(note::arrange(ty, doc, &created, &now)),
                    });
                }
            }
        }
        if !stranded.is_empty() {
            return Err(Error::InUse {
                code: "term_in_use",
                message: format!(
                    "the new taxonomy {name:?} drops terms that notes hold; keep them or rename them"
                ),
                by: stranded,
            });
        }
        let saved =
            self.commit_definitions(inner, vec![(path.clone(), Some(text))], changes, author)?;
        Ok((self.vault.read_text(&path)?, saved))
    }

    /// Delete taxonomy `name`, unless a type's field uses it.
    pub fn delete_taxonomy(&self, name: &str) -> Result<()> {
        let _w = self.write_lock();
        let inner = self.lock();
        if inner.schema.taxonomy(name).is_none() {
            return Err(missing_taxonomy(name));
        }
        let by: Vec<String> = inner
            .schema
            .types()
            .flat_map(|t| {
                t.fields.iter().filter_map(move |f| match &f.kind {
                    FieldKind::Taxonomy { taxonomy } if taxonomy == name => {
                        Some(format!("{} (field {})", type_path(&t.name), f.name))
                    }
                    _ => None,
                })
            })
            .collect();
        if !by.is_empty() {
            return Err(Error::InUse {
                code: "taxonomy_in_use",
                message: format!("taxonomy {name:?} cannot be deleted"),
                by,
            });
        }
        let author = Author::Session(crate::ADOPTED_BY.to_owned());
        self.commit_definitions(
            inner,
            vec![(taxonomy_path(name), None)],
            Vec::new(),
            &author,
        )?;
        Ok(())
    }

    /// The ids of every indexed note of `type_name`.
    fn ids_of_type(&self, inner: &Inner, type_name: &str) -> Result<Vec<String>> {
        let conn = inner.index.connection();
        let mut stmt = conn
            .prepare("SELECT id FROM notes WHERE type = ?1 ORDER BY id")
            .map_err(index::Error::from)?;
        let rows = stmt
            .query_map([type_name], |r| r.get::<_, String>(0))
            .map_err(index::Error::from)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(index::Error::from)?);
        }
        Ok(out)
    }

    // ---- scripts --------------------------------------------------------------------------

    /// The name of every script, sorted.
    pub fn scripts(&self) -> Result<Vec<String>> {
        let root = self.vault.root().join("scripts");
        let entries = std::fs::read_dir(&root).map_err(|source| {
            Error::Store(store::Error::Io {
                path: root.clone(),
                source,
            })
        })?;
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let file = e.file_name().to_string_lossy().into_owned();
                file.strip_suffix(".ws")
                    .filter(|n| schema::is_name(n))
                    .map(str::to_owned)
            })
            .collect();
        names.sort();
        Ok(names)
    }

    /// The source of `scripts/<name>.ws`.
    pub fn read_script(&self, name: &str) -> Result<String> {
        check_name("script", name)?;
        match self.vault.read_text(&script_path(name)) {
            Err(store::Error::NotFound(_)) => Err(missing_script(name)),
            other => Ok(other?),
        }
    }

    /// Create or replace `scripts/<name>.ws`, stored as sent.
    pub fn put_script(&self, name: &str, source: &str) -> Result<String> {
        let _w = self.write_lock();
        let _inner = self.lock();
        check_name("script", name)?;
        self.vault
            .commit(vec![Change::write(script_path(name), source.as_bytes())])?;
        Ok(source.to_owned())
    }

    /// Why `source` would not compile as script `name`, once for each way the vault runs it: as
    /// a `before_*` or `after_*` handler of some type, or as a task. A script nothing names yet is
    /// checked as a task. Empty when it compiles every way it runs. Nothing is stored.
    pub fn script_problems(&self, name: &str, source: &str) -> Result<Vec<ScriptProblem>> {
        check_name("script", name)?;
        let mut roles: Vec<ScriptRole> = self.read_index(|_, schema| {
            schema
                .types()
                .flat_map(|t| t.handlers.iter())
                .filter(|h| h.script == name)
                .map(|h| {
                    if h.event.starts_with("before_") {
                        ScriptRole::Before
                    } else {
                        ScriptRole::After
                    }
                })
                .collect()
        });
        for (_, text) in self.definitions("tasks").unwrap_or_default() {
            if let Ok(doc) = Doc::parse(&text, "task.wcl")
                && doc.field("script").and_then(Value::as_str) == Some(name)
            {
                roles.push(ScriptRole::Task);
            }
        }
        if roles.is_empty() {
            roles.push(ScriptRole::Task);
        }
        roles.sort();
        roles.dedup();
        Ok(roles
            .into_iter()
            .filter_map(|role| {
                self.hooks
                    .check_script(role, name, source)
                    .err()
                    .map(|message| ScriptProblem { role, message })
            })
            .collect())
    }

    /// Delete `scripts/<name>.ws`, unless a type's handler or a task names it.
    pub fn delete_script(&self, name: &str) -> Result<()> {
        let _w = self.write_lock();
        let inner = self.lock();
        self.read_script(name)?;
        let mut by: Vec<String> = inner
            .schema
            .types()
            .flat_map(|t| {
                t.handlers
                    .iter()
                    .filter(|h| h.script == name)
                    .map(|h| format!("{} (handler {:?})", type_path(&t.name), h.event))
            })
            .collect();
        for (task, text) in self.definitions("tasks")? {
            let file = task_path(&task);
            if let Ok(doc) = Doc::parse(&text, &file)
                && doc.field("script").and_then(Value::as_str) == Some(name)
            {
                by.push(file);
            }
        }
        if !by.is_empty() {
            return Err(Error::InUse {
                code: "script_in_use",
                message: format!("script {name:?} cannot be deleted"),
                by,
            });
        }
        self.vault.commit(vec![Change::delete(script_path(name))])?;
        Ok(())
    }

    // ---- tasks ----------------------------------------------------------------------------

    /// The `(name, text)` of every `tasks/<name>.wcl`, sorted by name.
    pub fn task_files(&self) -> Result<Vec<(String, String)>> {
        let mut tasks: Vec<_> = self
            .definitions("tasks")?
            .into_iter()
            .filter(|(n, _)| schema::is_name(n))
            .collect();
        tasks.sort();
        Ok(tasks)
    }

    /// The text of `tasks/<name>.wcl`.
    pub fn read_task(&self, name: &str) -> Result<String> {
        check_name("task", name)?;
        match self.vault.read_text(&task_path(name)) {
            Err(store::Error::NotFound(_)) => Err(missing_task(name)),
            other => Ok(other?),
        }
    }

    /// Create or replace `tasks/<name>.wcl` in canonical form. `check` validates the canonical
    /// text, which the `scheduler` crate knows how to read; the script the task names must exist.
    pub fn put_task(
        &self,
        name: &str,
        source: &str,
        check: impl FnOnce(&str) -> std::result::Result<(), Invalid>,
    ) -> Result<String> {
        let _inner = self.lock();
        check_name("task", name)?;
        let path = task_path(name);
        let text = store::canonical(source, &path)?;
        check(&text).map_err(Error::Invalid)?;
        let doc = Doc::parse(&text, &path).map_err(Error::Invalid)?;
        if let Some(script) = doc.field("script").and_then(Value::as_str)
            && self.read_script(script).is_err()
        {
            return Err(Error::Invalid(Invalid {
                file: path,
                issues: vec![Issue::new(
                    "script",
                    format!("there is no script {script:?}"),
                )],
            }));
        }
        self.vault
            .commit(vec![Change::write(path, text.as_bytes())])?;
        Ok(text)
    }

    /// Delete `tasks/<name>.wcl`.
    pub fn delete_task(&self, name: &str) -> Result<()> {
        let _inner = self.lock();
        self.read_task(name)?;
        self.vault.commit(vec![Change::delete(task_path(name))])?;
        Ok(())
    }

    // ---- canvases -------------------------------------------------------------------------

    /// Canvas `name` of note `id` as WCL, with the note's version. `name` is a canvas field, or
    /// `<content field>.<n>` for the n-th canvas block, counting from 1, inside a content field.
    pub fn canvas(&self, id: &str, name: &str) -> Result<(String, String)> {
        let note = self.read(id)?;
        let block = find_canvas(&note.doc, name)
            .ok_or_else(|| missing_canvas(id, name))?
            .clone();
        Ok((
            note.version,
            print_block(&block, &note_path(&note.type_name, id))?,
        ))
    }

    /// Replace canvas `name` of note `id` with the `canvas` block in `source`. The note must be at
    /// version `base`. A canvas field the note does not hold yet is added.
    pub fn put_canvas(
        &self,
        id: &str,
        name: &str,
        source: &str,
        base: Option<&str>,
        author: &Author,
    ) -> Result<Saved> {
        let _w = self.write_lock();
        let inner = self.lock();
        let (type_name, old) = self.existing(id)?;
        let path = note_path(&type_name, id);
        let submitted = Input::Wcl(source.to_owned()).parse(&path)?;
        let mut block = match (submitted.fields.is_empty(), submitted.blocks.as_slice()) {
            (true, [b]) if b.kind == "canvas" => b.clone(),
            _ => {
                return Err(Error::BadRequest(
                    "send exactly one `canvas { ... }` block".to_owned(),
                ));
            }
        };
        let mut doc = Input::Wcl(old.clone()).parse(&path)?;
        match name.split_once('.') {
            None => {
                let is_canvas_field = inner
                    .schema
                    .type_def(&type_name)
                    .and_then(|t| t.field(name))
                    .is_some_and(|f| f.kind == FieldKind::Canvas);
                if !is_canvas_field {
                    return Err(missing_canvas(id, name));
                }
                block.labels = vec![name.to_owned()];
                match doc
                    .blocks
                    .iter_mut()
                    .find(|b| b.kind == "canvas" && b.label() == Some(name))
                {
                    Some(slot) => *slot = block,
                    None => doc.blocks.push(block),
                }
            }
            Some(_) => {
                block.labels.clear();
                let slot =
                    find_canvas_mut(&mut doc, name).ok_or_else(|| missing_canvas(id, name))?;
                *slot = block;
            }
        }
        let change = self.prepare_put(&inner, id, &type_name, doc, Some(&old), author)?;
        let check = || self.check_base(id, &type_name, &old, base);
        self.apply(inner, vec![change], &check, author)
            .map(crate::first)
    }
}

/// The canvas a canvas name points at in a note.
fn find_canvas<'a>(doc: &'a Doc, name: &str) -> Option<&'a DocBlock> {
    match name.split_once('.') {
        None => doc
            .blocks
            .iter()
            .find(|b| b.kind == "canvas" && b.label() == Some(name)),
        Some((field, n)) => {
            let n: usize = n.parse().ok().filter(|n| *n > 0)?;
            doc.blocks
                .iter()
                .find(|b| b.kind == "content" && b.label() == Some(field))?
                .body
                .blocks_of("canvas")
                .nth(n - 1)
        }
    }
}

fn find_canvas_mut<'a>(doc: &'a mut Doc, name: &str) -> Option<&'a mut DocBlock> {
    let (field, n) = name.split_once('.')?;
    let n: usize = n.parse().ok().filter(|n| *n > 0)?;
    doc.blocks
        .iter_mut()
        .find(|b| b.kind == "content" && b.label() == Some(field))?
        .body
        .blocks
        .iter_mut()
        .filter(|b| b.kind == "canvas")
        .nth(n - 1)
}

fn print_block(block: &DocBlock, file: &str) -> Result<String> {
    let doc = Doc {
        fields: Vec::new(),
        blocks: vec![block.clone()],
    };
    Ok(store::canonical(&note::print(&doc), file)?)
}

fn missing_template(type_name: &str, name: &str) -> Error {
    Error::Missing(format!("type {type_name:?} has no template {name:?}"))
}

fn missing_taxonomy(name: &str) -> Error {
    Error::Missing(format!("there is no taxonomy {name:?}"))
}

fn missing_script(name: &str) -> Error {
    Error::Missing(format!("there is no script {name:?}"))
}

fn missing_task(name: &str) -> Error {
    Error::Missing(format!("there is no task {name:?}"))
}

fn missing_canvas(id: &str, name: &str) -> Error {
    Error::Missing(format!("note {id} has no canvas {name:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_take_their_children_and_prefer_the_longest_match() {
        let renames: BTreeMap<String, String> = [
            ("Active".to_owned(), "Doing".to_owned()),
            ("Active/Blocked".to_owned(), "Blocked".to_owned()),
        ]
        .into();
        assert_eq!(rename_term("Active", &renames).as_deref(), Some("Doing"));
        assert_eq!(
            rename_term("Active/In progress", &renames).as_deref(),
            Some("Doing/In progress")
        );
        assert_eq!(
            rename_term("Active/Blocked/Waiting", &renames).as_deref(),
            Some("Blocked/Waiting")
        );
        assert_eq!(rename_term("Activeish", &renames), None);
        assert_eq!(rename_term("Done", &renames), None);
    }
}
