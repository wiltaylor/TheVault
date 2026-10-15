//! Notifications for the user: what scripts, agents and the server want the user to see, under
//! the web app's bell. They live in `state.db`; each can open a note or a page of the app.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use state::{Author, NewNotification, Notification};
use writer::Writer;

use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

/// The page size when a request names none.
const DEFAULT_LIMIT: usize = 50;

/// A notification as the API sends it. `note_type` and `note_title` describe the note it opens
/// as the note is now, `null` when it names none or the note has gone.
fn item_json(w: &Writer, n: &Notification) -> Result<Value, ApiError> {
    let found = match &n.note_id {
        Some(id) => w.type_and_title(id)?,
        None => None,
    };
    let (note_type, note_title) = found.map_or((None, None), |(t, title)| (Some(t), Some(title)));
    Ok(json!({
        "id": n.id,
        "created": n.created_ms,
        "title": n.title,
        "body": n.body,
        "note": n.note_id,
        "note_type": note_type,
        "note_title": note_title,
        "url": n.url,
        "source": n.source,
        "read": n.read_ms,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    #[serde(default)]
    unread: Option<bool>,
    limit: Option<usize>,
    cursor: Option<String>,
}

/// `GET /api/notifications?unread=&limit=&cursor=`: a page of notifications, newest first, with
/// `next` for the page after and `unread`, the count of unread ones.
pub async fn list(State(w): Shared, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let out = blocking(w, move |w| {
        let page = w.state().notifications(
            q.unread.unwrap_or(false),
            q.limit.unwrap_or(DEFAULT_LIMIT),
            q.cursor.as_deref().filter(|c| !c.is_empty()),
        )?;
        let unread = w.state().unread_notifications()?;
        let items = page
            .items
            .iter()
            .map(|n| item_json(w, n))
            .collect::<Result<Vec<_>, _>>()?;
        Ok::<_, ApiError>(json!({ "items": items, "next": page.next, "unread": unread }))
    })
    .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Who a notification made through the API is from: `token:<name>` for an API token, `api` for
/// the browser.
fn source_of(caller: &Author) -> String {
    match caller {
        Author::Token(name) => format!("token:{name}"),
        _ => "api".to_owned(),
    }
}

/// `POST /api/notifications`: add a notification from `{title, body?, note?, url?}`. A note that
/// does not exist or is in the trash is a 400.
pub async fn create(
    State(w): Shared,
    Caller(caller): Caller,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let out = blocking(w, move |w| {
        let n = w.notify(&NewNotification {
            title: body.title,
            body: body.body,
            note_id: body.note,
            url: body.url,
            source: source_of(&caller),
        })?;
        item_json(w, &n)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// `POST /api/notifications/{id}/read`: mark one read. Answers with it and the unread count.
pub async fn read(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let out = blocking(w, move |w| {
        let n = w.state().mark_notification_read(&id)?;
        let mut out = item_json(w, &n)?;
        out["unread"] = json!(w.state().unread_notifications()?);
        Ok::<_, ApiError>(out)
    })
    .await?;
    Ok(Json(out))
}

/// `POST /api/notifications/read-all`: mark every notification read.
pub async fn read_all(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let marked = blocking(w, |w| w.state().mark_all_notifications_read()).await?;
    Ok(Json(json!({ "marked": marked, "unread": 0 })))
}

/// `DELETE /api/notifications/{id}`: dismiss one notification, read or not.
pub async fn delete(State(w): Shared, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let out = blocking(w, move |w| {
        w.state().delete_notification(&id)?;
        Ok::<_, ApiError>(json!({
            "id": id,
            "deleted": true,
            "unread": w.state().unread_notifications()?,
        }))
    })
    .await?;
    Ok(Json(out))
}

/// `DELETE /api/notifications`: remove every read notification.
pub async fn clear_read(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let out = blocking(w, |w| {
        let cleared = w.state().clear_read_notifications()?;
        Ok::<_, ApiError>(json!({
            "cleared": cleared,
            "unread": w.state().unread_notifications()?,
        }))
    })
    .await?;
    Ok(Json(out))
}
