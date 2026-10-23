//! The turns form of the AI chat and the apply route (ai D23).
//!
//! `POST /api/ai/chat` streams its answer in the response to the POST, which a client that
//! buffers request bodies (the Android webview's IPC) cannot read as it comes. The turns form
//! splits it in two:
//!
//! - `POST /api/ai/chat/turns` takes the body `POST /api/ai/chat` takes, starts the turn and
//!   answers 201 `{ turn }`.
//! - `GET /api/ai/chat/turns/{turn}/events` streams that turn's events, the same events as
//!   `POST /api/ai/chat` sends (see [`crate::ai_chat`]), and is usable with `EventSource`. A turn
//!   has one reader: a second GET is a 409.
//! - `DELETE /api/ai/chat/turns/{turn}` stops it, 204.
//!
//! Turns are held in memory, by id, for the caller that started them: another caller's turn is a
//! 404. A turn nobody reads within [`READER_WAIT`], or whose reader goes away, stops as a
//! disconnected `POST /api/ai/chat` does. At most [`MAX_TURNS`] run per caller at once; one more
//! is a 429 `too_many_turns`. Both forms share one turn runner, [`ai_chat::start`].
//!
//! `POST /api/ai/apply` applies a change the chat proposed, its body the `proposal` or
//! `type_proposal` event's data as the client received it (a type proposal is told apart by its
//! WCL `after` text), or a `taxonomy_proposal`'s (its `kind` is `taxonomy`, ai D31), through
//! [`ai_chat::apply_change`], the code the confirmation switches use (ai D22), as the caller,
//! version-checked against its `base_version`. It needs `write`, and `admin` for a type or
//! taxonomy change: without it, 403 `needs_admin`. It answers `{op, note_id, version}` for a
//! note, `{op, type_name}` for a type and `{op, taxonomy_name, notes}` for a taxonomy.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::Event;
use axum::{Extension, Json};
use serde_json::{Value, json};
use state::{Author, Scope};
use tokio::sync::{mpsc, watch};

use crate::ApiError;
use crate::ai_chat::{self, ChatBody};
use crate::ai_settings::Ai;
use crate::auth::{Caller, GrantedScope};

/// How long a started turn waits for its reader before it stops.
pub const READER_WAIT: Duration = Duration::from_secs(30);

/// The most turns one caller runs at once.
pub const MAX_TURNS: usize = 4;

/// The message of a type change applied without `admin`.
pub const NEEDS_ADMIN_MESSAGE: &str = "Apply type changes in the web app.";

/// The 403 a taxonomy change without `admin` answers (ai D31).
pub const NEEDS_ADMIN_TAXONOMY_MESSAGE: &str = "Apply taxonomy changes in the web app.";

/// The turns started and not yet over or read to the end, by id.
#[derive(Default)]
pub struct Turns {
    live: Mutex<HashMap<String, Held>>,
}

/// One turn held for its reader.
struct Held {
    owner: Author,
    /// The turn's events, until the reader takes them.
    rx: Option<mpsc::Receiver<Event>>,
    /// Set true to stop the turn.
    stop: watch::Sender<bool>,
    /// False once the turn is over.
    running: Arc<AtomicBool>,
}

impl Turns {
    fn live(&self) -> MutexGuard<'_, HashMap<String, Held>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Forget turn `id`, stopping it if it still runs.
    fn drop_turn(&self, id: &str) {
        if let Some(held) = self.live().remove(id) {
            let _ = held.stop.send(true);
        }
    }
}

fn unknown_turn() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_turn",
        "there is no such chat turn: it ended, was stopped, or waited too long for its reader",
    )
}

/// `POST /api/ai/chat/turns`.
pub async fn start(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Json(body): Json<ChatBody>,
) -> Result<impl IntoResponse, ApiError> {
    let id = state::random_hex(16);
    let (stop, stop_rx) = watch::channel(false);
    {
        let live = a.turns.live();
        let running = live
            .values()
            .filter(|h| h.owner == author && h.running.load(Ordering::SeqCst))
            .count();
        if running >= MAX_TURNS {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_turns",
                format!("{MAX_TURNS} chat turns are running already; stop one or wait for it"),
            ));
        }
    }
    let (rx, running) = ai_chat::start(a.writer.clone(), author.clone(), scope, body, stop_rx)?;
    a.turns.live().insert(
        id.clone(),
        Held {
            owner: author,
            rx: Some(rx),
            stop,
            running,
        },
    );
    // A turn nobody reads stops.
    let waiting = a.clone();
    let turn = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(READER_WAIT).await;
        let unread = waiting
            .turns
            .live()
            .get(&turn)
            .is_some_and(|h| h.rx.is_some());
        if unread {
            waiting.turns.drop_turn(&turn);
        }
    });
    Ok((StatusCode::CREATED, Json(json!({ "turn": id }))))
}

/// Forgets its turn when the reader's stream ends or the reader goes.
struct Reading {
    turns: Arc<Ai>,
    id: String,
}

impl Drop for Reading {
    fn drop(&mut self) {
        self.turns.turns.drop_turn(&self.id);
    }
}

/// `GET /api/ai/chat/turns/{turn}/events`.
pub async fn events(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rx = {
        let mut live = a.turns.live();
        let held = live
            .get_mut(&id)
            .filter(|h| h.owner == author)
            .ok_or_else(unknown_turn)?;
        held.rx.take().ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "turn_read",
                "this chat turn is being read already",
            )
        })?
    };
    let reading = Reading {
        turns: a.clone(),
        id,
    };
    let events = futures_util::stream::unfold((rx, reading), |(mut rx, reading)| async move {
        let e = rx.recv().await?;
        Some((Ok::<_, std::convert::Infallible>(e), (rx, reading)))
    });
    Ok(ai_chat::sse(events))
}

/// `DELETE /api/ai/chat/turns/{turn}`.
pub async fn stop(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mine = a.turns.live().get(&id).is_some_and(|h| h.owner == author);
    if !mine {
        return Err(unknown_turn());
    }
    a.turns.drop_turn(&id);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/ai/apply`.
pub async fn apply(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Extension(GrantedScope(scope)): Extension<GrantedScope>,
    Json(change): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !change.is_object() {
        return Err(ApiError::bad_request(
            "send the proposal as the chat sent it",
        ));
    }
    // A taxonomy proposal says so in `kind` (ai D31); a type proposal's `after` is the type
    // file's WCL text; a note proposal's is a note.
    let event = if change["kind"] == "taxonomy" {
        "taxonomy_proposal"
    } else if change["after"].is_string() && change.get("op").is_none() {
        "type_proposal"
    } else {
        "proposal"
    };
    if !scope.allows(Scope::Admin) {
        let message = match event {
            "type_proposal" => Some(NEEDS_ADMIN_MESSAGE),
            "taxonomy_proposal" => Some(NEEDS_ADMIN_TAXONOMY_MESSAGE),
            _ => None,
        };
        if let Some(message) = message {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "needs_admin", message));
        }
    }
    let w = a.writer.clone();
    tokio::task::spawn_blocking(move || ai_chat::apply_change(&w, &author, event, &change))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map(Json)
        .map_err(ApiError::from)
}
