//! Settings > Sync: the git sync connection between this instance and the owner's others
//! (git-sync D14, D18 to D20, D23, D24). Under `/api/integrations`, so every route needs the
//! admin scope.
//!
//! - `GET /api/integrations/gitsync` answers `{ configured, settings, token_set, ssh_key_set,
//!   ssh_public_key, host_fingerprint, key_set, running, last_run, last_error, last_report }`.
//!   `settings` is `null` before any were saved. No secret value is ever in it.
//! - `PUT .../settings` saves the settings object; a key left out takes its default. The export
//!   query must parse and run against this vault's schema, the import query must parse and must
//!   not use `this` (D23), and the schedule must be a five-field cron expression. A 400 names
//!   the setting at fault in `field`.
//! - `PUT .../token` takes `{ token }`; `DELETE .../token` removes it (D24).
//! - `POST .../ssh-key/generate` makes an ed25519 key pair, and `PUT .../ssh-key` takes
//!   `{ private_key }`, an unencrypted OpenSSH key; both store the private key and answer
//!   `{ public_key }` (D19). A new key forgets the pinned host key.
//! - `POST .../key/generate` makes a new sync key and answers `{ key }`, the only time it is
//!   shown; `PUT .../key` takes `{ key }`, a `tvsync1-` code (D14).
//! - `POST .../run` starts a round and answers 202, or 409 while one runs.
//! - `DELETE /api/integrations/gitsync` disconnects: every `gitsync_*` secret, the clone and the
//!   synced notes' bases go; the notes stay.
//!
//! The POSTs that take no input accept an empty body or JSON `null`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use gitsync::settings::{
    KEY, KNOWN_HOSTS, SSH_KEY, TOKEN, load_secrets, load_status, save_settings,
};
use gitsync::{GitSync, Settings, SyncKey, sshkey};
use serde::Deserialize;
use serde_json::{Value, json};
use writer::Writer;

use crate::ApiError;

type Shared = State<Arc<GitSync>>;

/// Run blocking work against the connection off the async runtime.
async fn on<T: Send + 'static>(
    g: &Arc<GitSync>,
    f: impl FnOnce(&GitSync) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let g = g.clone();
    tokio::task::spawn_blocking(move || f(&g))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
}

impl From<gitsync::Error> for ApiError {
    fn from(e: gitsync::Error) -> ApiError {
        match e {
            gitsync::Error::State(e) => e.into(),
            gitsync::Error::Writer(e) => e.into(),
            gitsync::Error::BadKey(_) | gitsync::Error::SshKey(_) => {
                ApiError::bad_request(e.to_string())
            }
            e => internal(e),
        }
    }
}

fn json_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    body.map(|Json(b)| b)
        .map_err(|e| ApiError::bad_request(e.body_text()))
}

/// Secrets need the server's secret key (OQ-S2).
fn need_key(w: &Writer) -> Result<(), ApiError> {
    if w.state().has_secret_key() {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "secret_key",
            "the server has no secret key, so git sync cannot store its settings",
        ))
    }
}

fn delete_secret(w: &Writer, name: &str) -> Result<(), ApiError> {
    match w.state().delete_secret(name) {
        Ok(()) | Err(state::Error::UnknownSecret(_)) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The page's view of the connection.
fn status_of(g: &GitSync) -> Result<Value, ApiError> {
    let w = g.writer();
    // A round saves its status before it clears the flag, so the flag read first means a round
    // seen as ended has its status saved.
    let running = g.running();
    let settings = g.settings()?;
    let secrets = load_secrets(w.state())?;
    let status = load_status(w.state())?;
    let public_key = secrets
        .ssh_key
        .as_deref()
        .and_then(|k| sshkey::public_key_of(k).ok());
    Ok(json!({
        "configured": g.configured()?,
        "settings": settings,
        "token_set": secrets.token.is_some(),
        "ssh_key_set": secrets.ssh_key.is_some(),
        "ssh_public_key": public_key,
        "host_fingerprint": status.host_fingerprint,
        "key_set": secrets.key.is_some(),
        "running": running,
        "last_run": status.last_run,
        "last_error": status.last_error,
        "last_report": status.last_report,
    }))
}

pub async fn status(State(g): Shared) -> Result<Json<Value>, ApiError> {
    on(&g, status_of).await.map(Json)
}

/// A 400 naming the setting at fault.
fn refused(field: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        body: json!({ "error": "invalid_settings", "field": field, "message": message.into() }),
    }
}

/// Check `s` as the route says.
fn check(w: &Writer, s: &Settings) -> Result<(), ApiError> {
    if !s.export_query.trim().is_empty() {
        let q = query::parse(&s.export_query).map_err(|e| {
            refused(
                "export_query",
                format!(
                    "the export query does not parse at line {}, column {}: {}",
                    e.line, e.column, e.message
                ),
            )
        })?;
        let page = query::PageRequest {
            cursor: None,
            size: 1,
        };
        w.read_index(|index, schema| query::run(&q, &query::Context::new(index, schema), &page))
            .map_err(|e| match e {
                query::Error::Index(_) | query::Error::Sqlite(_) => internal(e),
                e => refused("export_query", format!("the export query cannot run: {e}")),
            })?;
    }
    if !s.import_query.trim().is_empty() {
        let q = query::parse(&s.import_query).map_err(|e| {
            refused(
                "import_query",
                format!(
                    "the import query does not parse at line {}, column {}: {}",
                    e.line, e.column, e.message
                ),
            )
        })?;
        if q.filter.as_ref().is_some_and(query::Expr::uses_this) {
            return Err(refused(
                "import_query",
                "the import query cannot use `this`: it runs against other instances' notes, \
                 not from a note",
            ));
        }
    }
    schema::parse_cron(&s.schedule).map_err(|e| refused("schedule", e))?;
    Ok(())
}

pub async fn put_settings(
    State(g): Shared,
    body: Result<Json<Settings>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let settings = json_body(body)?;
    on(&g, move |g| {
        let w = g.writer();
        need_key(w)?;
        let settings = settings.completed(&g.folder_name());
        check(w, &settings)?;
        save_settings(w.state(), &settings)?;
        status_of(g)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenBody {
    token: String,
}

pub async fn put_token(
    State(g): Shared,
    body: Result<Json<TokenBody>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let b = json_body(body)?;
    let token = b.token.trim().to_owned();
    if token.is_empty() {
        return Err(ApiError::bad_request("the token is empty"));
    }
    on(&g, move |g| {
        need_key(g.writer())?;
        Ok(g.writer().state().set_secret(TOKEN, &token)?)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_token(State(g): Shared) -> Result<StatusCode, ApiError> {
    on(&g, |g| delete_secret(g.writer(), TOKEN)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Store `private` as the SSH key and forget the pinned host, which belongs to the old key's
/// setup.
fn store_ssh_key(g: &GitSync, private: &str) -> Result<(), ApiError> {
    let w = g.writer();
    w.state().set_secret(SSH_KEY, private)?;
    delete_secret(w, KNOWN_HOSTS)?;
    let mut status = load_status(w.state())?;
    if status.host_fingerprint.take().is_some() {
        gitsync::settings::save_status(w.state(), &status)?;
    }
    Ok(())
}

pub async fn generate_ssh_key(State(g): Shared) -> Result<Json<Value>, ApiError> {
    on(&g, |g| {
        need_key(g.writer())?;
        let comment = format!("thevault-gitsync-{}", g.folder_name());
        let (private, public) = sshkey::generate_ed25519(&comment)?;
        store_ssh_key(g, &private)?;
        Ok(json!({ "public_key": public }))
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshKeyBody {
    private_key: String,
}

pub async fn put_ssh_key(
    State(g): Shared,
    body: Result<Json<SshKeyBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let b = json_body(body)?;
    on(&g, move |g| {
        need_key(g.writer())?;
        let mut private = b.private_key.trim().to_owned();
        private.push('\n');
        let public = sshkey::public_key_of(&private)?;
        store_ssh_key(g, &private)?;
        Ok(json!({ "public_key": public }))
    })
    .await
    .map(Json)
}

pub async fn generate_key(State(g): Shared) -> Result<Json<Value>, ApiError> {
    on(&g, |g| {
        need_key(g.writer())?;
        let key = SyncKey::generate().to_text();
        g.writer().state().set_secret(KEY, &key)?;
        Ok(json!({ "key": key }))
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBody {
    key: String,
}

pub async fn put_key(
    State(g): Shared,
    body: Result<Json<KeyBody>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let b = json_body(body)?;
    let key = SyncKey::parse(&b.key)?.to_text();
    on(&g, move |g| {
        need_key(g.writer())?;
        Ok(g.writer().state().set_secret(KEY, &key)?)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run(State(g): Shared) -> Result<(StatusCode, Json<Value>), ApiError> {
    let started = on(&g, |g| {
        if !g.configured()? {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "not_configured",
                "git sync needs a remote URL and a sync key first",
            ));
        }
        Ok(())
    })
    .await
    .map(|()| g.start())?;
    if !started {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "running",
            "a git sync round is already running",
        ));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "started": true }))))
}

pub async fn disconnect(State(g): Shared) -> Result<StatusCode, ApiError> {
    on(&g, |g| Ok(g.disconnect()?)).await?;
    Ok(StatusCode::NO_CONTENT)
}
