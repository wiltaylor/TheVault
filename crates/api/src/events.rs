//! `GET /api/events`: a server-sent event stream of the vault's live events for an open client
//! (D4). Each event names what changed by id; the client fetches the details through the API.
//!
//! | Event | Data |
//! |---|---|
//! | `notification.created` | `{id, unread}` |
//! | `notification.updated` | `{id, unread}`: an unread notification's title or body changed |
//! | `notification.read` | `{id, unread}`; `id` is `null` when every one was marked read |
//! | `notification.dismissed` | `{id, unread}` |
//! | `notification.cleared` | `{cleared, unread}` |
//! | `note.changed` | `{id, type, version, deleted}` |
//! | `device.command` | `{device, id}` |
//! | `device.settings` | `{device}` |
//! | `clients.updated` | `{clients: [{kind, arch, version, build}]}`: every app build listed now |
//! | `ai_turn` | `{agent, thread, turn, running}`: a chat turn on an agent's thread started or ended (agents G17); `turn` is `null` for a trigger's run |
//! | `activity` | `{item}`: an entry of the activity log, as `GET /api/activity` lists it (activity Y3); a coalesced `note.update` entry comes again with the same `id` and replaces it. Only `admin` callers get these |
//!
//! Each event's SSE `id` is its sequence number since the server started. A comment line goes
//! out every [`HEARTBEAT`] so proxies keep the connection open. A client that falls too far
//! behind, or a server shutting down, ends the stream; the client reconnects and fetches what it
//! shows again.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, HeaderName};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream;
use serde_json::json;
use state::{LiveEvent, Scope};
use writer::Writer;

use crate::auth::GrantedScope;

/// How often a heartbeat comment goes out on an idle stream.
pub const HEARTBEAT: Duration = Duration::from_secs(15);

/// The SSE event for `event`, numbered `seq`.
fn sse(seq: u64, event: &LiveEvent) -> Event {
    let data = match event {
        LiveEvent::NotificationCreated { id, unread }
        | LiveEvent::NotificationUpdated { id, unread }
        | LiveEvent::NotificationDismissed { id, unread } => json!({ "id": id, "unread": unread }),
        LiveEvent::NotificationRead { id, unread } => json!({ "id": id, "unread": unread }),
        LiveEvent::NotificationCleared { cleared, unread } => {
            json!({ "cleared": cleared, "unread": unread })
        }
        LiveEvent::NoteChanged {
            id,
            type_name,
            version,
            deleted,
            ..
        } => json!({ "id": id, "type": type_name, "version": version, "deleted": deleted }),
        LiveEvent::DeviceCommand { device, id } => json!({ "device": device, "id": id }),
        LiveEvent::DeviceSettings { device } => json!({ "device": device }),
        LiveEvent::ClientsUpdated { clients } => json!({
            "clients": clients
                .iter()
                .map(|c| json!({
                    "kind": c.kind,
                    "arch": c.arch,
                    "version": c.version,
                    "build": c.build,
                }))
                .collect::<Vec<_>>()
        }),
        LiveEvent::AiTurn {
            agent,
            thread,
            turn,
            running,
        } => json!({ "agent": agent, "thread": thread, "turn": turn, "running": running }),
        LiveEvent::Activity { item } => json!({ "item": crate::activity::item_json(item) }),
    };
    Event::default()
        .event(event.name())
        .id(seq.to_string())
        .data(data.to_string())
}

/// `GET /api/events`: the live event stream.
/// The activity log is admin only (activity Y3), so its events go only to `admin` callers.
pub async fn stream(
    State(w): State<Arc<Writer>>,
    scope: Option<Extension<GrantedScope>>,
) -> impl IntoResponse {
    let admin = scope.is_some_and(|Extension(GrantedScope(s))| s == Scope::Admin);
    let sub = w.state().events().subscribe();
    // A first comment, so the client knows the stream is open before any event comes.
    let opened = stream::once(async { Ok::<_, Infallible>(Event::default().comment("open")) });
    let events = stream::unfold(sub, move |mut sub| async move {
        loop {
            let (seq, event) = sub.recv().await.ok()?;
            if !admin && matches!(event, LiveEvent::Activity { .. }) {
                continue;
            }
            return Some((Ok::<_, Infallible>(sse(seq, &event)), sub));
        }
    });
    let body = Sse::new(futures_util::StreamExt::chain(opened, events))
        .keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat"));
    (
        [
            (CACHE_CONTROL, "no-cache"),
            (HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        body,
    )
}
