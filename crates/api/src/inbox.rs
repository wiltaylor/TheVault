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

/// Session-only counted preview and single-use approval for one recommendation batch.
pub async fn triage(
    State(w): State<Arc<Writer>>,
    Caller(author): Caller,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let state::Author::Session(principal) = author else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "session_required",
            "Sign in to preview mail triage",
        ));
    };
    blocking(w, move |w| {
        let bad = |s: String| ApiError::bad_request(s);
        let scheduler = crate::script_tools::scheduler_for(w).ok_or_else(|| bad("Home server required".into()))?;
        let agent = body["agent"].as_str().ok_or_else(|| bad("Choose an agent".into()))?;
        let lane = scheduler::agents::MAIL_TRIAGE;
        if let Some(id) = body["preview"].as_str() {
            let raw = w.state().confirm_trigger_backfill(id, &principal, agent, lane)?
                .ok_or_else(|| bad("Preview expired or consumed".into()))?;
            let notes: Vec<String> = serde_json::from_str(&raw).map_err(|e| bad(e.to_string()))?;
            let queued = scheduler.mail_triage_enqueue(agent, &notes).map_err(bad)?;
            w.state().record_activity(state::NewActivity::new("mail.triage", "notes", &state::Author::Session(principal), format!("Read-only recommendation batch: {} notes, queued={queued}", notes.len())));
            Ok(Json(json!({"queued":queued})))
        } else {
            let limit = body["limit"].as_u64().filter(|n| (20..=50).contains(n)).ok_or_else(|| bad("Batch size must be 20 to 50".into()))? as usize;
            let mut notes = scheduler.mail_triage_candidates(agent).map_err(bad)?;
            let count = notes.len();
            notes.truncate(limit);
            if notes.len() < 20 { return Err(bad("At least 20 eligible messages are required. Held notes must first be classified or released in Held Inbox.".into())); }
            let preview = w.state().preview_trigger_backfill(&principal, agent, lane, &serde_json::to_string(&notes).expect("notes"))?;
            Ok(Json(json!({"count":count,"selected":notes.len(),"preview":preview})))
        }
    }).await
}

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
