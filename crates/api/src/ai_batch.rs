//! Current-selection approval only. A preview is an expiring, session-bound snapshot,
//! never an automation rule. Confirmation goes through individual Apply for every item.
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{Extension, Json, extract::State, http::{HeaderMap, StatusCode}};
use serde::Deserialize;
use serde_json::{Value, json};
use state::Author;

use crate::{
    ApiError,
    agent_threads::{self, Stored},
    ai_settings::Ai,
    auth::{Caller, GrantedScope},
};

#[derive(Clone)]
pub(crate) struct Preview {
    owner: Author,
    session: String,
    created: Instant,
    items: Vec<Stored>,
}

#[derive(Deserialize)]
pub struct Selection {
    thread: String,
    proposal: String,
}

#[derive(Deserialize)]
pub struct Confirmation {
    preview: String,
    confirmed: bool,
}

fn session(author: &Author) -> Result<(), ApiError> {
    if matches!(author, Author::Session(_)) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "human_only",
            "Apply this kind needs a session.",
        ))
    }
}

/// Separate event, operation and note type. Never group unknown/missing note types.
fn kind(w: &writer::Writer, p: &Stored) -> Option<(String, String, String)> {
    if p.event == "applied" || !crate::ai_chat::PROPOSAL_EVENTS.contains(&p.event.as_str()) {
        return None;
    }
    // Type changes predate the explicit op field. Absence is not always an update.
    let implicit = if p.event == "type_proposal" && p.data["before"].is_null() {
        "create"
    } else {
        "update"
    };
    let op = p.data["op"].as_str().unwrap_or(implicit).to_owned();
    let ty = if p.event == "proposal" {
        if op == "create" {
            p.data["type"].as_str()?.to_owned()
        } else {
            w.read(p.data["note_id"].as_str()?).ok()?.type_name
        }
    } else {
        String::new()
    };
    Some((p.event.clone(), op, ty))
}

pub async fn preview(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    headers: HeaderMap,
    Json(b): Json<Selection>,
) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    let w = a.writer.clone();
    let (items, group) = crate::blocking(w, move |w| {
        if agent_threads::running(&b.thread).is_some() {
            return Err(ApiError::bad_request("Wait for the thread to finish."));
        }
        let t = agent_threads::Thread::read(w, &b.thread)?;
        let seed = t
            .transcript
            .proposals
            .iter()
            .find(|p| p.id() == b.proposal && p.waits())
            .ok_or_else(|| ApiError::bad_request("Select a pending proposal."))?;
        let group = kind(w, seed)
            .ok_or_else(|| ApiError::bad_request("This proposal cannot be grouped."))?;
        let items: Vec<_> = t
            .transcript
            .proposals
            .iter()
            .filter(|p| {
                p.waits()
                    && !p.id().is_empty()
                    && p.data["thread"] == b.thread
                    && kind(w, p).as_ref() == Some(&group)
            })
            .cloned()
            .collect();
        if items.is_empty() || items.len() > 100 {
            return Err(ApiError::bad_request(
                "Preview needs between 1 and 100 proposals.",
            ));
        }
        Ok((items, group))
    })
    .await?;
    let token = state::random_hex(24);
    let response = json!({"preview":token, "count":items.len(), "kind":group,
        "items":items.iter().map(|p| json!({"id":p.id(),
            "summary":agent_threads::proposal_what(&p.event, &p.data), "data":p.data})).collect::<Vec<_>>()});
    let mut previews = a
        .approval_previews
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    previews.retain(|_, p| p.created.elapsed() < Duration::from_secs(600));
    if previews.len() >= 100 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "preview_limit",
            "Too many previews. Try later.",
        ));
    }
    previews.insert(
        token,
        Preview {
            owner: author,
            session: session_key(&headers),
            created: Instant::now(),
            items,
        },
    );
    Ok(Json(response))
}

pub async fn confirm(
    State(a): State<Arc<Ai>>,
    Caller(author): Caller,
    Extension(scope): Extension<GrantedScope>,
    headers: HeaderMap,
    Json(b): Json<Confirmation>,
) -> Result<Json<Value>, ApiError> {
    session(&author)?;
    if !b.confirmed {
        return Err(ApiError::bad_request("Explicit confirmation is required."));
    }
    let preview = a
        .approval_previews
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&b.preview)
        .filter(|p| p.owner == author && p.session == session_key(&headers) && p.created.elapsed() < Duration::from_secs(600))
        .cloned()
        .ok_or_else(|| ApiError::bad_request("Preview expired. Preview again."))?;
    let mut results = Vec::new();
    for p in preview.items {
        let result = crate::ai_turns::apply(
            State(a.clone()),
            Caller(author.clone()),
            Extension(GrantedScope(scope.0)),
            Json(p.data.clone()),
        )
        .await;
        results.push(match result {
            Ok(Json(value)) => json!({"id":p.id(), "ok":true, "result":value}),
            Err(e) => json!({"id":p.id(), "ok":false, "error":e.body}),
        });
    }
    Ok(Json(json!({"results":results})))
}

fn session_key(headers: &HeaderMap) -> String {
    // HTTP session callers were authenticated before this handler. The embedded owner
    // router has no cookie and is one local owner session, not a remote API credential.
    crate::auth::session_id(headers).unwrap_or_else(|| "embedded-owner".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_type_create_and_update_are_different_kinds() {
        let root = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(root.path()).unwrap();
        let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
        let w = writer::Writer::open(vault, state).unwrap();
        let mut p = Stored {
            event: "type_proposal".into(),
            data: json!({"id":"type", "before":null, "after":"label = \"New\""}),
            status: agent_threads::Status::Pending,
            result: None,
            error: None,
            reported: false,
        };
        let create = kind(&w, &p).unwrap();
        assert_eq!(create.1, "create");
        p.data["before"] = json!("label = \"Old\"");
        let update = kind(&w, &p).unwrap();
        assert_eq!(update.1, "update");
        assert_ne!(create, update);
        p.event = "applied".into();
        assert!(kind(&w, &p).is_none());
    }
}
