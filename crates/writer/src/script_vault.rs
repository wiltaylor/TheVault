//! The write path as scripts see it: [`Vault`] backed by a [`Writer`], so every change a script
//! makes goes through it (D30).

use std::collections::BTreeMap;
use std::path::PathBuf;

use schema::{FieldKind, Taxonomy, Term};
use script_core::Vault;
use serde_json::{Value as Json, json};
use state::Author;

use crate::{Archived, Input, Saved, Writer, doc_to_json};

fn err(e: impl ToString) -> String {
    e.to_string()
}

fn syntax(e: query::ParseError) -> String {
    format!(
        "query syntax error at line {}, column {}: {}",
        e.line, e.column, e.message
    )
}

fn run_page(w: &Writer, q: &query::Query, page: query::PageRequest) -> Result<query::Page, String> {
    w.read_index(|index, schema| query::run(q, &query::Context::new(index, schema), &page))
        .map_err(err)
}

fn link_rows(w: &Writer, sql: &str, id: &str) -> Result<Vec<Json>, String> {
    w.read(id).map_err(err)?;
    w.read_index(|index, _| {
        let mut stmt = index.connection().prepare(sql)?;
        let rows = stmt.query_map([id], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "type": r.get::<_, Option<String>>(1)?,
                "title": r.get::<_, Option<String>>(2)?,
                "via": r.get::<_, String>(3)?,
                "field": r.get::<_, String>(4)?,
            }))
        })?;
        rows.collect::<rusqlite::Result<Vec<Json>>>()
    })
    .map_err(err)
}

fn asset_json(a: &crate::Asset) -> Json {
    json!({ "id": a.id, "filename": a.filename, "size": a.size })
}

fn term_child<'a>(terms: &'a mut [Term], name: &str) -> Option<&'a mut Term> {
    terms.iter_mut().find(|t| t.name == name)
}

/// `t` with the term at `path` added. `Ok(false)` when it is there already.
fn grow(t: &mut Taxonomy, path: &str) -> Result<bool, String> {
    if t.contains(path) {
        return Ok(false);
    }
    let names: Vec<&str> = path.split('/').collect();
    if names.iter().any(|n| n.trim().is_empty()) {
        return Err(format!(
            "{path:?} is not a term path: a name in it is empty"
        ));
    }
    let (last, parents) = names.split_last().ok_or("the term path is empty")?;
    let mut level = &mut t.terms;
    for (i, name) in parents.iter().enumerate() {
        level = &mut term_child(level, name)
            .ok_or_else(|| {
                format!(
                    "taxonomy {:?} has no term {:?}, so {path:?} cannot be added under it",
                    t.name,
                    parents[..=i].join("/")
                )
            })?
            .children;
    }
    level.push(Term {
        name: (*last).to_owned(),
        colour: None,
        description: None,
        children: Vec::new(),
    });
    Ok(true)
}

/// The write path itself: every change a script makes goes through `writer` (D30).
impl Vault for Writer {
    fn get(&self, id: &str) -> Result<Json, String> {
        let n = self.read(id).map_err(err)?;
        Ok(json!({
            "id": n.id,
            "type": n.type_name,
            "version": n.version,
            "note": doc_to_json(&n.doc),
        }))
    }

    fn query(&self, text: &str, max_rows: usize) -> Result<Vec<Json>, String> {
        let q = query::parse(text).map_err(syntax)?;
        let mut rows = Vec::new();
        let mut cursor = None;
        loop {
            let page = run_page(
                self,
                &q,
                query::PageRequest {
                    cursor: cursor.take(),
                    size: query::MAX_PAGE,
                },
            )?;
            if page.total > max_rows {
                return Err(format!(
                    "the query matched {} notes, more than the {max_rows} a script may read; \
                     add a limit or use vault::query_page",
                    page.total
                ));
            }
            for row in page.rows {
                rows.push(serde_json::to_value(row).map_err(err)?);
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(rows),
            }
        }
    }

    fn query_page(&self, text: &str, size: usize, cursor: Option<String>) -> Result<Json, String> {
        let q = query::parse(text).map_err(syntax)?;
        let page = run_page(
            self,
            &q,
            query::PageRequest {
                cursor,
                size: size.clamp(1, query::MAX_PAGE),
            },
        )?;
        Ok(json!({
            "rows": page.rows,
            "total": page.total,
            "next": page.next,
        }))
    }

    fn search(&self, text: &str, limit: usize, archived: Archived) -> Result<Vec<Json>, String> {
        if text.trim().is_empty() {
            return Err("the search text is empty".into());
        }
        let q = query::Query::search_all(text, archived);
        let page = run_page(
            self,
            &q,
            query::PageRequest {
                cursor: None,
                size: limit.clamp(1, query::MAX_PAGE),
            },
        )?;
        page.rows
            .into_iter()
            .map(|r| serde_json::to_value(r).map_err(err))
            .collect()
    }

    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String> {
        Writer::create(self, type_name, &Input::Json(note), author)
            .map(saved)
            .map_err(err)
    }

    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String> {
        Writer::update(self, id, &Input::Json(note), Some(version), author)
            .map(saved)
            .map_err(err)
    }

    fn delete(&self, id: &str, version: &str, author: &Author) -> Result<Json, String> {
        Writer::delete(self, id, Some(version), author)
            .map(saved)
            .map_err(err)
    }

    fn restore(&self, id: &str, author: &Author) -> Result<Json, String> {
        self.restore_from_trash(id, author).map(saved).map_err(err)
    }

    fn set_archived(&self, id: &str, archived: bool, author: &Author) -> Result<Json, String> {
        let version = self
            .current_version(id)
            .map_err(err)?
            .ok_or_else(|| format!("note {id} does not exist"))?;
        Writer::set_archived(self, id, archived, Some(&version), author)
            .map(saved)
            .map_err(err)
    }

    fn links(&self, id: &str) -> Result<Vec<Json>, String> {
        link_rows(
            self,
            "SELECT l.dst, n.type, n.title, l.via, l.field FROM links l \
             LEFT JOIN notes n ON n.id = l.dst WHERE l.src = ?1 ORDER BY l.via, l.field, l.pos",
            id,
        )
    }

    fn backlinks(&self, id: &str) -> Result<Vec<Json>, String> {
        link_rows(
            self,
            "SELECT l.src, n.type, n.title, l.via, l.field FROM links l \
             JOIN notes n ON n.id = l.src WHERE l.dst = ?1 ORDER BY n.title, l.src, l.field",
            id,
        )
    }

    fn assets(&self) -> Result<Vec<Json>, String> {
        Ok(Writer::assets(self)
            .map_err(err)?
            .iter()
            .map(asset_json)
            .collect())
    }

    fn asset(&self, id: &str) -> Result<(Json, Vec<u8>), String> {
        let (a, bytes) = Writer::asset(self, id).map_err(err)?;
        Ok((asset_json(&a), bytes))
    }

    fn upload_asset(&self, filename: &str, bytes: Vec<u8>) -> Result<Json, String> {
        self.put_asset(filename, bytes)
            .map(|a| asset_json(&a))
            .map_err(err)
    }

    fn types(&self) -> Result<Vec<Json>, String> {
        Ok(self
            .schema()
            .types()
            .map(|t| {
                let fields: Vec<Json> = t
                    .fields
                    .iter()
                    .map(|f| {
                        let mut out = json!({
                            "name": f.name,
                            "label": f.label,
                            "kind": f.kind.name(),
                            "required": f.required,
                            "multiple": f.multiple,
                            "computed": f.computed,
                        });
                        if let FieldKind::Taxonomy { taxonomy } = &f.kind {
                            out["taxonomy"] = json!(taxonomy);
                        }
                        out
                    })
                    .collect();
                json!({
                    "name": t.name,
                    "label": t.label,
                    "plural": t.plural,
                    "description": t.description,
                    "fields": fields,
                })
            })
            .collect())
    }

    fn taxonomies(&self) -> Result<Vec<Json>, String> {
        Ok(self
            .schema()
            .taxonomies()
            .map(|t| {
                json!({
                    "name": t.name,
                    "label": t.label,
                    "description": t.description,
                    "allow_new": t.allow_new,
                    "terms": t.paths(),
                })
            })
            .collect())
    }

    fn add_term(&self, taxonomy: &str, path: &str, author: &Author) -> Result<Json, String> {
        let mut t = self
            .schema()
            .taxonomy(taxonomy)
            .cloned()
            .ok_or_else(|| format!("there is no taxonomy {taxonomy:?}"))?;
        let added = grow(&mut t, path)?;
        if added {
            if !t.allow_new {
                return Err(format!(
                    "taxonomy {taxonomy:?} is closed: only an open taxonomy (allow_new = true) \
                     takes new terms from a script"
                ));
            }
            self.put_taxonomy(taxonomy, &t.to_wcl(), &BTreeMap::new(), author)
                .map_err(err)?;
        }
        Ok(json!({ "taxonomy": taxonomy, "path": path, "added": added }))
    }

    fn daily(&self, date: &str, author: &Author) -> Result<Json, String> {
        let (n, created) = self.daily_note(date, author).map_err(err)?;
        Ok(json!({
            "id": n.id,
            "type": n.type_name,
            "version": n.version,
            "note": doc_to_json(&n.doc),
            "created": created,
        }))
    }

    fn secret(&self, name: &str) -> Result<Option<String>, String> {
        self.state().get_secret(name).map_err(err)
    }

    fn notify(&self, source: &str, title: &str, body: &str, note: &str) -> Result<Json, String> {
        let n = Writer::notify(
            self,
            &state::NewNotification {
                title: title.to_owned(),
                body: (!body.is_empty()).then(|| body.to_owned()),
                note_id: (!note.is_empty()).then(|| note.to_owned()),
                url: None,
                source: source.to_owned(),
            },
        )
        .map_err(err)?;
        Ok(json!({
            "id": n.id,
            "title": n.title,
            "body": n.body,
            "note": n.note_id,
            "source": n.source,
        }))
    }

    fn protected(&self) -> Vec<PathBuf> {
        vec![self.vault().root().to_path_buf()]
    }

    fn now_ms(&self) -> i64 {
        Writer::now_ms(self)
    }
}

fn saved(s: Saved) -> Json {
    let note = Input::Wcl(s.wcl)
        .parse(&crate::note_path(&s.type_name, &s.id))
        .map(|d| doc_to_json(&d))
        .unwrap_or(Json::Null);
    json!({
        "id": s.id,
        "type": s.type_name,
        "version": s.version,
        "note": note,
    })
}
