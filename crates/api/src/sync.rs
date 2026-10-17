//! What the desktop and Android apps sync through: the version handshake, the change feed, and the
//! base versions each device still holds (D20, D27, D29).
//!
//! `GET /api/sync/info` needs no credentials, so an app can tell whether it can sync before it
//! pairs. `GET /api/sync/changes` pages through every note and synced file that changed after a
//! cursor, each once at its latest change, with its content as it is now. A device pushes its own
//! writes through the normal notes API with `If-Match`.
//!
//! What the server enforces for a device's `write` token:
//!
//! - Every note write it makes is a version of its own. It never merges into the browser's
//!   version before it, and the browser's next save does not merge into it.
//! - Types, templates, taxonomies, scripts and tasks need `admin`, so it gets 403 writing any of
//!   them. It reads them, and the feed carries them, like any other token.
//! - It sets the push endpoint and bases of its own device only; an admin may set any device's.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{Author, ChangeKey, Journaled, Scope};
use writer::Writer;

use crate::auth::GrantedScope;
use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// How many entries a page of the feed holds when the request names no limit.
const DEFAULT_LIMIT: i64 = 500;

/// The most entries a page of the feed may ask for.
const MAX_LIMIT: i64 = 5000;

/// `GET /api/sync/info`: the protocol and vault format versions, and the server's own version.
pub async fn info() -> Json<Value> {
    Json(json!({
        "protocol": vocab::SYNC_PROTOCOL,
        "format_version": vocab::FORMAT_VERSION,
        "server_version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /api/version`: the server's own version, for Settings > General's About section. Like
/// `/api/sync/info` it needs no credentials.
pub async fn version() -> Json<Value> {
    Json(json!({ "server_version": env!("CARGO_PKG_VERSION") }))
}

#[derive(Deserialize)]
pub struct ChangesQuery {
    since: Option<i64>,
    limit: Option<i64>,
}

/// One feed entry, with the content of its note or file as it is now.
fn entry(w: &Writer, c: &Journaled) -> Result<Value, ApiError> {
    let state = w.state();
    match &c.key {
        ChangeKey::Note(id) => {
            if let Some(head) = state.head(id)? {
                return Ok(json!({
                    "seq": c.seq, "at": c.at_ms, "kind": "note", "id": id,
                    "type": head.note_type, "version": head.token, "wcl": head.content,
                    "deleted": false, "trashed": false,
                }));
            }
            let trashed = state.trash_entry(id)?;
            Ok(json!({
                "seq": c.seq, "at": c.at_ms, "kind": "note", "id": id,
                "type": trashed.as_ref().map(|t| &t.note_type),
                "version": trashed.as_ref().map(|t| &t.version),
                "wcl": null, "deleted": true, "trashed": trashed.is_some(),
            }))
        }
        ChangeKey::File(path) => {
            let (kind, name) = writer::synced_file(path).unwrap_or(("file", path.as_str()));
            let text = w.read_synced(path)?;
            Ok(json!({
                "seq": c.seq, "at": c.at_ms, "kind": kind, "path": path, "name": name,
                "deleted": text.is_none(), "text": text,
            }))
        }
    }
}

/// `GET /api/sync/changes?since=<seq>&limit=`: every note and synced file whose latest change
/// comes after `since`, oldest first, with `next`, the cursor for the next page, and `latest`, the
/// newest change there is. A note entry carries its type, version and WCL, or `deleted` and
/// whether it went to the trash; a file entry carries its kind, path, name and text, or `deleted`.
pub async fn changes(
    State(w): Shared,
    Query(q): Query<ChangesQuery>,
) -> Result<Json<Value>, ApiError> {
    let since = q.since.unwrap_or(0);
    if since < 0 {
        return Err(ApiError::bad_request("since must be 0 or more"));
    }
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::bad_request(format!(
            "limit must be from 1 to {MAX_LIMIT}"
        )));
    }
    let limit = usize::try_from(limit).unwrap_or(1);
    let page = blocking(w, move |w| -> Result<_, ApiError> {
        let page = w.state().changes_since(since, limit)?;
        let entries = page
            .changes
            .iter()
            .map(|c| entry(w, c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "changes": entries, "next": page.next, "latest": page.latest }))
    })
    .await?;
    Ok(Json(page))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasesBody {
    bases: BTreeMap<String, String>,
}

/// `PUT /api/devices/{id}/bases`: replace the base versions the device holds, `{"bases": {"<note
/// id>": "<version>"}}`, one per note: the oldest version a write it has queued is based on.
/// Pruning keeps them and a browser save does not merge into one (D29). `{"bases": {}}` clears
/// them. Only the device's own token, or an admin, may.
pub async fn put_bases(
    State(w): Shared,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Path(id): Path<String>,
    Json(body): Json<BasesBody>,
) -> Result<Json<Value>, ApiError> {
    let bases: Vec<(String, String)> = body.bases.into_iter().collect();
    let out = blocking(w, move |w| -> Result<_, ApiError> {
        let state = w.state();
        let device = state.device(&id)?;
        let own = matches!(&author, Author::Token(name) if *name == device.token_id);
        if !own && !scope.allows(Scope::Admin) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "only the device itself or an admin may set its bases",
            ));
        }
        state.set_device_bases(&id, &bases)?;
        let bases: BTreeMap<String, String> = state.device_bases(&id)?.into_iter().collect();
        Ok(json!({ "device_id": id, "bases": bases }))
    })
    .await?;
    Ok(Json(out))
}
