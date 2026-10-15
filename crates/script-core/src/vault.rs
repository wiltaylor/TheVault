//! What the `vault` and `secrets` host modules reach: notes, queries, search, links, assets,
//! types, taxonomies and secrets (D10, D30).

use std::path::PathBuf;

use serde_json::{Value as Json, json};
use vocab::{Archived, Author};

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

    /// One page of `text`: `{ rows, total, next }`, where `next` is the cursor of the next page
    /// or `null` on the last. `cursor` is `None` for the first page.
    fn query_page(&self, text: &str, size: usize, cursor: Option<String>) -> Result<Json, String>;

    /// Full-text search over every note, best match first, at most `limit` rows. `archived` says
    /// whether archived notes are left out, kept or the only ones searched.
    fn search(&self, text: &str, limit: usize, archived: Archived) -> Result<Vec<Json>, String>;

    /// Create a note of `type_name` from its JSON form.
    fn create(&self, type_name: &str, note: Json, author: &Author) -> Result<Json, String>;

    /// Replace note `id`, which must still be at `version` (D12).
    fn update(&self, id: &str, note: Json, version: &str, author: &Author) -> Result<Json, String>;

    /// Move note `id`, which must still be at `version`, to the trash.
    fn delete(&self, id: &str, version: &str, author: &Author) -> Result<Json, String>;

    /// Set some of note `id`'s fields, keeping the rest and its blocks. A `null` value removes
    /// the field. An empty `version` takes the note as it is now.
    fn patch(
        &self,
        id: &str,
        fields: Json,
        version: &str,
        author: &Author,
    ) -> Result<Json, String> {
        let Json::Object(fields) = fields else {
            return Err("patch takes a map of field names to values".into());
        };
        let (mut body, version) = body_at(self.get(id)?, version)?;
        let target = body
            .get_mut("fields")
            .and_then(Json::as_object_mut)
            .ok_or("the note has no fields")?;
        for (name, value) in fields {
            if value.is_null() {
                target.remove(&name);
            } else {
                target.insert(name, value);
            }
        }
        self.update(id, body, &version, author)
    }

    /// Append `blocks` to note `id`'s content field `field`, starting the field when the note
    /// does not hold it yet. An empty `version` takes the note as it is now.
    fn append_blocks(
        &self,
        id: &str,
        field: &str,
        blocks: Vec<Json>,
        version: &str,
        author: &Author,
    ) -> Result<Json, String> {
        let (mut body, version) = body_at(self.get(id)?, version)?;
        let list = body
            .as_object_mut()
            .ok_or("the note is not an object")?
            .entry("blocks")
            .or_insert_with(|| Json::Array(Vec::new()))
            .as_array_mut()
            .ok_or("the note's blocks are not a list")?;
        let is_field = |b: &Json| {
            b["kind"] == "content" && b["labels"].as_array().is_some_and(|l| l == &[json!(field)])
        };
        let content = match list.iter().position(is_field) {
            Some(i) => &mut list[i],
            None => {
                list.push(json!({ "kind": "content", "labels": [field] }));
                list.last_mut().expect("just pushed")
            }
        };
        let inner = content
            .as_object_mut()
            .ok_or("the content block is not an object")?
            .entry("blocks")
            .or_insert_with(|| Json::Array(Vec::new()))
            .as_array_mut()
            .ok_or("the content block's blocks are not a list")?;
        inner.extend(blocks);
        self.update(id, body, &version, author)
    }

    /// Take note `id` out of the trash.
    fn restore(&self, id: &str, author: &Author) -> Result<Json, String>;

    /// Archive note `id` as it is now, or take it out of the archive when `archived` is false.
    fn set_archived(&self, id: &str, archived: bool, author: &Author) -> Result<Json, String>;

    /// The notes note `id` links to: `[{ id, type, title, via, field }]`, where `via` is `field`
    /// for a reference field and `mention` for a `[[link]]` in text. `type` and `title` are
    /// `null` for a link to a note that does not exist.
    fn links(&self, id: &str) -> Result<Vec<Json>, String>;

    /// The notes that link to note `id`, in the same form as [`Vault::links`].
    fn backlinks(&self, id: &str) -> Result<Vec<Json>, String>;

    /// Every asset: `[{ id, filename, size }]`.
    fn assets(&self) -> Result<Vec<Json>, String>;

    /// Asset `id`: `{ id, filename, size }` and its bytes.
    fn asset(&self, id: &str) -> Result<(Json, Vec<u8>), String>;

    /// Store `bytes` as a new asset named `filename`.
    fn upload_asset(&self, filename: &str, bytes: Vec<u8>) -> Result<Json, String>;

    /// Every type: `[{ name, label, plural, description, fields: [{ name, label, kind,
    /// required, multiple, computed, taxonomy? }] }]`.
    fn types(&self) -> Result<Vec<Json>, String>;

    /// Every taxonomy: `[{ name, label, description, allow_new, terms: [path] }]`.
    fn taxonomies(&self) -> Result<Vec<Json>, String>;

    /// Add the term at `path` to open taxonomy `taxonomy`. Only the last name in the path may be
    /// new. Adding a term the taxonomy holds already changes nothing. Returns `{ taxonomy, path,
    /// added }`.
    fn add_term(&self, taxonomy: &str, path: &str, author: &Author) -> Result<Json, String>;

    /// The daily note for `date` (`YYYY-MM-DD`), created when there is none. Returns the note
    /// with `created: bool`.
    fn daily(&self, date: &str, author: &Author) -> Result<Json, String>;

    /// The value of secret `name`, or `None` when there is none (D10).
    fn secret(&self, name: &str) -> Result<Option<String>, String>;

    /// Notify the user: store a notification from `source`, such as `task:<name>`, opening note
    /// `note` when it is not empty. Returns `{ id, title, body, note, source }`.
    fn notify(
        &self,
        _source: &str,
        _title: &str,
        _body: &str,
        _note: &str,
    ) -> Result<Json, String> {
        Err("notifications are not available here".into())
    }

    /// Directories no script may reach through `fs`, whatever its task allows: the vault.
    fn protected(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// The vault's time now, in milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// A note's body and the version to write it over: `version`, or the note's own when empty.
fn body_at(note: Json, version: &str) -> Result<(Json, String), String> {
    let version = if version.is_empty() {
        note["version"]
            .as_str()
            .ok_or("the note has no version")?
            .to_owned()
    } else {
        version.to_owned()
    };
    let body = note.get("note").cloned().ok_or("the note has no body")?;
    Ok((body, version))
}
