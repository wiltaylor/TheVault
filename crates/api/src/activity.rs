//! `GET /api/activity`: the activity log (activity Y3), for the Log page. Admin only, since the
//! log shows everything that happens in the vault.
//!
//! The API's own recording points, such as logins, tokens and AI chat turns, call
//! [`state::State::record_activity`] through the helpers here; none of them fails its request.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{Activity, ActivityFilter};
use writer::Writer;

use crate::{ApiError, blocking};

/// An entry as the API sends it, in the list and in the `activity` live event.
pub fn item_json(a: &Activity) -> Value {
    json!({
        "id": a.id,
        "at_ms": a.at_ms,
        "kind": a.kind,
        "category": a.category,
        "actor_kind": a.actor_kind,
        "actor": a.actor,
        "summary": a.summary,
        "subject_kind": a.subject_kind,
        "subject": a.subject,
        "detail": a.detail,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    category: Option<String>,
    actor: Option<String>,
    kind: Option<String>,
    q: Option<String>,
    before: Option<String>,
    limit: Option<usize>,
}

/// `GET /api/activity?category=&actor=&kind=&q=&before=&limit=`: a page of entries, newest
/// first, with `next`, the `before` of the page after, or `null` on the last.
pub async fn list(
    State(w): State<Arc<Writer>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    if let Some(c) = q.category.as_deref().filter(|c| !c.is_empty())
        && !state::ACTIVITY_CATEGORIES.contains(&c)
    {
        return Err(ApiError::bad_request(format!(
            "unknown category {c:?}; expected one of {}",
            state::ACTIVITY_CATEGORIES.join(", ")
        )));
    }
    let filter = ActivityFilter {
        category: q.category,
        actor: q.actor,
        kind: q.kind,
        q: q.q,
        before: q.before,
        limit: q.limit,
    };
    let page = blocking(w, move |w| w.state().activity(&filter)).await?;
    Ok(Json(json!({
        "items": page.items.iter().map(item_json).collect::<Vec<_>>(),
        "next": page.next,
    })))
}
