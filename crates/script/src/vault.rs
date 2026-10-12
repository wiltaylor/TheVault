//! What the `vault` and `secrets` host modules reach: notes, queries and secrets (D10, D30).

use serde_json::{Value as Json, json};
use state::Author;
use writer::{Input, Saved, Writer, doc_to_json};

/// The vault as scripts see it. Every error is a message the script receives as an `Err`.
///
/// Notes travel in the JSON form of the notes API: `{ id, type, version, note }`, where `note`
/// is `{ fields, blocks }` (D32).
pub trait Vault: Send + Sync {
    /// Note `id` as it is now.
    fn get(&self, id: &str) -> Result<Json, String>;

    /// Every row of `text`, at most `max_rows` of them. A result with more rows is an error, so a
    /// script never acts on a silently truncated list.
    fn query(&self, text: &str, max_rows: usize) -> Result<Vec<Json>, String>;

    /// Create a note of `type_name` from its JSON form.
    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String>;

    /// Replace note `id`, which must still be at `version` (D12).
    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String>;

    /// Move note `id`, which must still be at `version`, to the trash.
    fn delete(&self, id: &str, version: &str, author: &Author) -> Result<Json, String>;

    /// The value of secret `name`, or `None` when there is none (D10).
    fn secret(&self, name: &str) -> Result<Option<String>, String>;
}

/// The write path itself: every change a script makes goes through `writer` (D30).
impl Vault for Writer {
    fn get(&self, id: &str) -> Result<Json, String> {
        let n = self.read(id).map_err(|e| e.to_string())?;
        Ok(json!({
            "id": n.id,
            "type": n.type_name,
            "version": n.version,
            "note": doc_to_json(&n.doc),
        }))
    }

    fn query(&self, text: &str, max_rows: usize) -> Result<Vec<Json>, String> {
        let q = query::parse(text).map_err(|e| {
            format!(
                "query syntax error at line {}, column {}: {}",
                e.line, e.column, e.message
            )
        })?;
        let mut rows = Vec::new();
        let mut cursor = None;
        loop {
            let page = query::PageRequest {
                cursor: cursor.take(),
                size: query::MAX_PAGE,
            };
            let page = self
                .read_index(|index, schema| {
                    query::run(&q, &query::Context::new(index, schema), &page)
                })
                .map_err(|e| e.to_string())?;
            if page.total > max_rows {
                return Err(format!(
                    "the query matched {} notes, more than the {max_rows} a script may read; \
                     add a limit",
                    page.total
                ));
            }
            for row in page.rows {
                rows.push(serde_json::to_value(row).map_err(|e| e.to_string())?);
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(rows),
            }
        }
    }

    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String> {
        Writer::create(self, type_name, &Input::Json(note), author)
            .map(saved)
            .map_err(|e| e.to_string())
    }

    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String> {
        Writer::update(self, id, &Input::Json(note), Some(version), author)
            .map(saved)
            .map_err(|e| e.to_string())
    }

    fn delete(&self, id: &str, version: &str, author: &Author) -> Result<Json, String> {
        Writer::delete(self, id, Some(version), author)
            .map(saved)
            .map_err(|e| e.to_string())
    }

    fn secret(&self, name: &str) -> Result<Option<String>, String> {
        self.state().get_secret(name).map_err(|e| e.to_string())
    }
}

fn saved(s: Saved) -> Json {
    let note = Input::Wcl(s.wcl)
        .parse(&writer::note_path(&s.type_name, &s.id))
        .map(|d| doc_to_json(&d))
        .unwrap_or(Json::Null);
    json!({
        "id": s.id,
        "type": s.type_name,
        "version": s.version,
        "note": note,
    })
}
