//! `POST /api/inbox/mark-processed` (TV-30): the inbox's one-time catch-up, which marks every
//! email, message and call sent or made before a date as processed ([`writer::Writer::mark_processed_before`]).
//!
//! Body: `{ "before": "<RFC 3339 datetime>", "dry_run"?: bool }`. The answer:
//! `{ "dry_run", "matched": { "email", "message", "call" }, "total", "changed", "chunks" }`.
//! A dry run counts what would change and changes nothing.
//!
//! Only the home server runs it. The apps' embedded server answers 409 `home_only`: a device
//! would write every note locally and sync each back, where the Gmail sync's own writes could
//! conflict with them. The home server's change reaches the apps by sync.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use writer::Writer;

use crate::{ApiError, Caller, blocking};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkBody {
    before: String,
    #[serde(default)]
    dry_run: bool,
}

/// `POST /api/inbox/mark-processed`.
pub async fn mark_processed(
    State(w): State<Arc<Writer>>,
    Caller(author): Caller,
    Json(body): Json<MarkBody>,
) -> Result<Json<Value>, ApiError> {
    if !w.is_home() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "home_only",
            "Marking older items processed runs on the home server only. Use the web app.",
        ));
    }
    let before = writer::instant(&body.before).ok_or_else(|| {
        ApiError::bad_request(format!(
            "`before` must be a datetime such as 2026-11-01T00:00:00Z, not {:?}",
            body.before
        ))
    })?;
    let dry_run = body.dry_run;
    let done = blocking(w, move |w| {
        w.mark_processed_before(before, dry_run, &author)
    })
    .await?;
    let matched: Map<String, Value> = done
        .matched
        .iter()
        .map(|(t, n)| (t.clone(), json!(n)))
        .collect();
    Ok(Json(json!({
        "dry_run": done.dry_run,
        "matched": matched,
        "total": done.total(),
        "changed": done.changed,
        "chunks": done.chunks,
    })))
}
