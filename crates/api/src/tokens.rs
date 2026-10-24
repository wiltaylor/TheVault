//! API token management (D11). Every route here needs `admin`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use state::{NewActivity, Scope, TokenInfo};
use writer::Writer;

use crate::{ApiError, Caller, blocking};

type Shared = State<Arc<Writer>>;

fn token_json(t: &TokenInfo) -> Value {
    json!({
        "name": t.name,
        "scope": t.scope.as_str(),
        "created": t.created_ms,
        "last_used": t.last_used_ms,
    })
}

/// Token names appear in history as authors, so keep them short and plain.
fn check_name(name: &str) -> Result<(), ApiError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "token name {name:?} must be 1 to 64 letters, digits, `-`, `_` or `.`"
        )))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    name: String,
    scope: String,
}

/// `POST /api/tokens`: create a token. The response is the only time its secret is shown.
pub async fn create(
    State(w): Shared,
    Caller(by): Caller,
    Json(body): Json<CreateBody>,
) -> Result<Response, ApiError> {
    check_name(&body.name)?;
    let scope: Scope = body.scope.parse()?;
    let (info, secret) = blocking(w, move |w| {
        let made = w.state().create_token(&body.name, scope)?;
        w.state().record_activity(
            NewActivity::new(
                "auth.token_created",
                "auth",
                &by,
                format!("Created the {} token {}", scope.as_str(), body.name),
            )
            .subject("token", &body.name)
            .detail(json!({ "name": body.name, "scope": scope.as_str() })),
        );
        Ok::<_, state::Error>(made)
    })
    .await?;
    let mut out = token_json(&info);
    out["secret"] = Value::String(secret);
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

/// `GET /api/tokens`: every token, without secrets.
pub async fn list(State(w): Shared) -> Result<Json<Value>, ApiError> {
    let tokens = blocking(w, |w| w.state().list_tokens()).await?;
    let tokens: Vec<Value> = tokens.iter().map(token_json).collect();
    Ok(Json(json!({ "tokens": tokens })))
}

/// `DELETE /api/tokens/{name}`: revoke a token. Its secret stops working at once.
pub async fn revoke(
    State(w): Shared,
    Caller(by): Caller,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    blocking(w, move |w| {
        w.state().revoke_token(&name)?;
        w.state().record_activity(
            NewActivity::new(
                "auth.token_revoked",
                "auth",
                &by,
                format!("Revoked the token {name}"),
            )
            .subject("token", &name)
            .detail(json!({ "name": name })),
        );
        Ok::<_, state::Error>(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
