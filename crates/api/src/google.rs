//! Settings > Integrations > Google: signing in with the user's own OAuth client, and what the
//! Integrations page shows (google-sync D1, D18, D19).
//!
//! - `POST /api/integrations/google/start` takes `{ client_id, client_secret, redirect }`, where
//!   `redirect` is `loopback` or `direct`. It stores the client id and secret as the secrets
//!   `google_client_id` and `google_client_secret`, holds a PKCE verifier and a state value for
//!   [`PENDING_FOR`], and answers `{ auth_url }`: Google's consent page asking for
//!   [`SCOPES`] with `access_type=offline` and `prompt=consent`. A `loopback` flow redirects to
//!   [`LOOPBACK_REDIRECT`], where nothing listens, so the user pastes the address the browser
//!   failed to load; a `direct` flow redirects to `<origin>/integrations/google/callback`, the
//!   web client's route, with the origin from the request's `Origin` header.
//! - `POST /api/integrations/google/finish` takes `{ url }`, that pasted or callback address. It
//!   checks the state, exchanges the code with the verifier at the token endpoint, stores
//!   `google_refresh_token`, reads the account's address from userinfo, writes it to the
//!   `google_account` secret and writes `google_settings` (the current settings, keeping the
//!   endpoints the start used, with `scopes`, the scopes Google granted), unpauses the `google_gmail` and `google_calendar` tasks that
//!   exist, and starts a run of each on its own thread. It answers once the runs are marked
//!   running, never waiting on them (ticket 113). An error from Google is a 400 carrying
//!   Google's message.
//! - `POST /api/integrations/google/disconnect` pauses the tasks, revokes the refresh token at
//!   Google, deletes the three credential secrets and `google_account`, keeps `google_settings`
//!   without its `scopes`, and clears the cursors in both state secrets, so a later connection
//!   lists everything again. A remote revocation failure does not stop local disconnect; the
//!   answer carries a sanitized `revocation_warning` instead of claiming Google revoked it.
//! - `PUT /api/integrations/google/settings` saves `{ gmail_mode, gmail_labels,
//!   gmail_window_days, gmail_backfill_to, calendars, default_calendar, gmail_remote_images }` into `google_settings`,
//!   `gmail_remote_images` defaults off and may only be supplied by an owner session.
//!   keeping its `endpoints`. `gmail_mode` is `labels` or `all`, the window 1 to 3650 days, the
//!   back-fill date `YYYY-MM-DD` or null, and `calendars` not empty.
//! - `GET /api/integrations/google` answers `{ connected, account, client_stored, can_send,
//!   can_modify, settings, gmail, calendar }`, `can_send` true when the connection granted
//!   [`SEND_SCOPE`] (outbox X4), `can_modify` when it granted [`MODIFY_SCOPE`] (TV-25), each side `{ task, installed, paused, running, last_run, last_error,
//!   summary }`, from the secrets of [`writer::google`] and the tasks. `paused` is `null` when the
//!   task does not exist; `running` is the scheduler's, true while a run is going; `last_run` is
//!   RFC 3339 in UTC ending `Z`; `summary` is the last line the latest finished run logged.
//!
//! No note holds any of it (google-sync D21). The token and userinfo endpoints come from the
//! settings' `endpoints` object when it names them, else Google's own (the test seam). A flow
//! keeps the endpoints it started with.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use scheduler::Scheduler;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use writer::Writer;
use writer::google::{ACCOUNT, CALENDAR_STATE, GMAIL_STATE, SETTINGS};

use crate::ApiError;

pub(crate) mod attachments;

/// The secrets the sign-in stores and the sync scripts read (google-sync D19).
pub const CLIENT_ID: &str = state::GOOGLE_CLIENT_ID;
pub const CLIENT_SECRET: &str = state::GOOGLE_CLIENT_SECRET;
pub const REFRESH_TOKEN: &str = state::GOOGLE_REFRESH_TOKEN;
const SECRETS: [&str; 3] = [CLIENT_ID, CLIENT_SECRET, REFRESH_TOKEN];

/// The tasks a connection unpauses and a disconnection pauses, with the side of the status each
/// is and the secret holding its state.
const TASKS: [(&str, &str, &str); 2] = [
    ("google_gmail", "gmail", GMAIL_STATE),
    ("google_calendar", "calendar", CALENDAR_STATE),
];

/// The scopes the consent page asks for. `gmail.modify` reads mail and lets Done archive and
/// delete trash a message (TV-25), `gmail.send` lets the Outbox send email (outbox X4).
pub const SCOPES: &str = "https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/gmail.send https://www.googleapis.com/auth/calendar openid email";

/// The scope the Outbox needs to send email (outbox X4).
pub const SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";

/// The scope mail actions need: archive on Done, trash on delete (TV-25).
pub const MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";

/// The key of `google_settings` holding the scopes the connection granted, as a list.
const SCOPES_KEY: &str = "scopes";

/// Google's consent page.
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google's token endpoint.
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Google's userinfo endpoint.
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
/// Google's token revocation endpoint.
const REVOCATION_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Where a loopback flow sends the browser: a fixed port on the user's own machine, where
/// nothing listens, so the browser shows an error page whose address holds the code.
pub const LOOPBACK_REDIRECT: &str = "http://127.0.0.1:53682/";

/// The web client's route a direct flow returns to.
const CALLBACK_PATH: &str = "/integrations/google/callback";

/// How long a started sign-in waits for its `finish`.
const PENDING_FOR: Duration = Duration::from_secs(10 * 60);

/// How long a request to Google may take.
const GOOGLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The connect routes' state: the scheduler, which holds the writer, and the sign-ins started.
pub struct Google {
    scheduler: Arc<Scheduler>,
    pending: Mutex<HashMap<String, Pending>>,
    http: reqwest::Client,
    connection: tokio::sync::Mutex<()>,
    attachment_fetch: tokio::sync::Semaphore,
}

impl Google {
    pub fn new(scheduler: Arc<Scheduler>) -> Google {
        Google {
            attachment_fetch: tokio::sync::Semaphore::new(1),
            scheduler,
            pending: Mutex::default(),
            connection: tokio::sync::Mutex::new(()),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(GOOGLE_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    fn writer(&self) -> &Arc<Writer> {
        self.scheduler.writer()
    }

    /// The started sign-ins, without those older than [`PENDING_FOR`].
    fn pending(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, v| v.started.elapsed() < PENDING_FOR);
        p
    }
}

type Shared = State<Arc<Google>>;

/// A sign-in waiting for its `finish`.
struct Pending {
    verifier: String,
    redirect_uri: String,
    endpoints: Endpoints,
    started: Instant,
}

/// Where the token and userinfo endpoints are, and the settings note's `endpoints` dict when it
/// had one.
#[derive(Clone)]
struct Endpoints {
    token: String,
    userinfo: String,
    revoke: String,
    dict: Option<Map<String, Value>>,
}

impl Endpoints {
    /// From the settings' `endpoints` object, when it has one.
    fn of(settings: &Map<String, Value>) -> Endpoints {
        let dict = settings
            .get("endpoints")
            .and_then(Value::as_object)
            .cloned();
        let get = |key: &str, default: &str| {
            dict.as_ref()
                .and_then(|d| d.get(key))
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_owned()
        };
        Endpoints {
            token: get("token", TOKEN_URL),
            userinfo: get("userinfo", USERINFO_URL),
            revoke: get("revoke", REVOCATION_URL),
            dict,
        }
    }
}

/// Run blocking work against the writer off the async runtime.
async fn on_writer<T: Send + 'static>(
    g: &Arc<Google>,
    f: impl FnOnce(&Writer) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let w = g.writer().clone();
    tokio::task::spawn_blocking(move || f(&w))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

/// The settings: the defaults with `google_settings` over them. The defaults alone when the
/// server has no secrets key.
fn settings(w: &Writer) -> Result<Map<String, Value>, ApiError> {
    if !w.state().has_secret_key() {
        return Ok(writer::google::default_settings());
    }
    Ok(writer::google::settings(w.state())?)
}

/// The JSON object in secret `name`, empty when there is none or no secrets key.
fn object(w: &Writer, name: &str) -> Result<Map<String, Value>, ApiError> {
    if !w.state().has_secret_key() {
        return Ok(Map::new());
    }
    Ok(writer::google::read_object(w.state(), name)?)
}

fn delete_secret(w: &Writer, name: &str) -> Result<(), ApiError> {
    match w.state().delete_secret(name) {
        Ok(()) | Err(state::Error::UnknownSecret(_)) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// 64 random hex digits: a PKCE verifier (43 to 128 characters) or a state value.
fn random_token() -> String {
    state::random_hex(32)
}

fn google_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "google", message)
}

/// `error: error_description` from the token endpoint's error body, or the body as it came.
fn describe_google_error(status: reqwest::StatusCode, body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(v) if v["error"].is_string() => match v["error_description"].as_str() {
            Some(d) => format!("Google answered {}: {d}", v["error"].as_str().unwrap_or("")),
            None => format!("Google answered {}", v["error"].as_str().unwrap_or("")),
        },
        Ok(v) if v["error"]["message"].is_string() => {
            format!(
                "Google answered {status}: {}",
                v["error"]["message"].as_str().unwrap_or("")
            )
        }
        _ => format!("Google answered {status}: {}", body.trim()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartBody {
    client_id: String,
    client_secret: String,
    redirect: String,
}

/// The origin the page was loaded from: the `Origin` header, or else the forwarded scheme and
/// the host.
fn origin(headers: &HeaderMap) -> Option<String> {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(o) = text(header::ORIGIN.as_str()).filter(|o| *o != "null") {
        return Some(o.trim_end_matches('/').to_owned());
    }
    let host = text(header::HOST.as_str())?;
    let proto = text("x-forwarded-proto").unwrap_or("http");
    Some(format!("{proto}://{host}"))
}

/// `POST /api/integrations/google/start`.
pub async fn start(
    State(g): Shared,
    headers: HeaderMap,
    Json(b): Json<StartBody>,
) -> Result<Json<Value>, ApiError> {
    let client_id = b.client_id.trim().to_owned();
    let client_secret = b.client_secret.trim().to_owned();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(ApiError::bad_request(
            "enter the OAuth client's id and secret from Google Cloud",
        ));
    }
    let redirect_uri = match b.redirect.as_str() {
        "loopback" => LOOPBACK_REDIRECT.to_owned(),
        "direct" => {
            let origin = origin(&headers)
                .ok_or_else(|| ApiError::bad_request("a direct sign-in needs the page's origin"))?;
            format!("{origin}{CALLBACK_PATH}")
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "redirect must be loopback or direct, not {other:?}"
            )));
        }
    };
    let id = client_id.clone();
    let endpoints = on_writer(&g, move |w| {
        w.state().set_secret(CLIENT_ID, &id)?;
        w.state().set_secret(CLIENT_SECRET, &client_secret)?;
        Ok(Endpoints::of(&settings(w)?))
    })
    .await?;

    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token();
    let auth_url = Url::parse_with_params(
        AUTH_URL,
        [
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("include_granted_scopes", "true"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ],
    )
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    g.pending().insert(
        state,
        Pending {
            verifier,
            redirect_uri,
            endpoints,
            started: Instant::now(),
        },
    );
    Ok(Json(json!({ "auth_url": auth_url.as_str() })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishBody {
    url: String,
}

/// `POST /api/integrations/google/finish`.
pub async fn finish(State(g): Shared, Json(b): Json<FinishBody>) -> Result<Json<Value>, ApiError> {
    let _connection = g.connection.lock().await;
    let url = Url::parse(b.url.trim()).map_err(|_| {
        ApiError::bad_request(
            "paste the whole address from the browser's address bar, starting http://127.0.0.1",
        )
    })?;
    let param = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    let pending = param("state").and_then(|s| g.pending().remove(&s));
    if let Some(error) = param("error") {
        return Err(google_error(format!(
            "Google answered {error}: the sign-in was not approved. Connect Google again."
        )));
    }
    let Some(pending) = pending else {
        return Err(ApiError::bad_request(
            "this sign-in was not started here, or it expired or was already used: Connect Google again",
        ));
    };
    let code = param("code")
        .ok_or_else(|| ApiError::bad_request("the address has no code: Connect Google again"))?;

    let (client_id, client_secret) = on_writer(&g, |w| {
        let s = w.state();
        Ok((s.get_secret(CLIENT_ID)?, s.get_secret(CLIENT_SECRET)?))
    })
    .await?;
    let (Some(client_id), Some(client_secret)) = (client_id, client_secret) else {
        return Err(ApiError::bad_request(
            "the OAuth client's id and secret are gone: Connect Google again",
        ));
    };

    // Exchange the code for the refresh token.
    let res = g
        .http
        .post(&pending.endpoints.token)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| google_error(format!("could not reach Google's token endpoint: {e}")))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(google_error(describe_google_error(status, &body)));
    }
    let tokens: Value = serde_json::from_str(&body)
        .map_err(|_| google_error("Google's token endpoint answered something other than JSON"))?;
    let Some(refresh) = tokens["refresh_token"].as_str().map(str::to_owned) else {
        return Err(google_error(
            "Google sent no refresh token: check the OAuth client is a Desktop or Web client and Connect Google again",
        ));
    };
    let access = tokens["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    // What the user granted: the consent screen lets them untick a scope.
    let granted: Vec<String> = tokens["scope"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    // Who signed in.
    let res = g
        .http
        .get(&pending.endpoints.userinfo)
        .bearer_auth(&access)
        .send()
        .await
        .map_err(|e| google_error(format!("could not reach Google's userinfo endpoint: {e}")))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(google_error(describe_google_error(status, &body)));
    }
    let info: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let Some(account) = info["email"].as_str().map(str::to_owned) else {
        return Err(google_error("Google did not say which account signed in"));
    };

    let dict = pending.endpoints.dict.clone();
    on_writer(&g, move |w| {
        w.state().set_secret(REFRESH_TOKEN, &refresh)?;
        w.state().set_secret(ACCOUNT, &account)?;
        let mut s = settings(w)?;
        match dict {
            Some(d) => s.insert("endpoints".to_owned(), Value::Object(d)),
            None => s.remove("endpoints"),
        };
        s.insert(SCOPES_KEY.to_owned(), json!(granted));
        writer::google::write_object(w.state(), SETTINGS, &s)?;
        Ok(())
    })
    .await?;
    // Unpause the tasks and start their first runs. Each run goes on its own thread and is
    // marked running before this answers, so the answer shows both sides syncing and never waits
    // on a run.
    let s = g.scheduler.clone();
    tokio::task::spawn_blocking(move || {
        for (task, _, _) in TASKS {
            if s.set_paused(task, false)? {
                match s.trigger_in_background(task) {
                    // A run already going is as good as this one.
                    Ok(_) | Err(scheduler::Error::Running(_)) => {}
                    Err(e) => eprintln!("google: starting {task}: {e}"),
                }
            }
        }
        Ok::<_, scheduler::Error>(())
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))??;
    status_of(&g).await.map(Json)
}

/// `POST /api/integrations/google/disconnect`.
pub async fn disconnect(State(g): Shared) -> Result<Json<Value>, ApiError> {
    // Finish must not install and unpause a new connection during this revocation.
    let _connection = g.connection.lock().await;
    // Stop new scheduled runs before revoking the credential they use.
    let s = g.scheduler.clone();
    tokio::task::spawn_blocking(move || {
        for (task, _, _) in TASKS {
            s.set_paused(task, true)?;
        }
        Ok::<_, scheduler::Error>(())
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))??;

    // Keep the credential in memory only for the revocation request. Neither the endpoint's
    // response nor reqwest's error reaches the caller: either can contain sensitive text.
    let credential = on_writer(&g, |w| {
        let refresh = w.state().get_secret(REFRESH_TOKEN)?;
        let revoke = Endpoints::of(&settings(w)?).revoke;
        Ok(refresh.map(|token| (token, revoke)))
    })
    .await;
    let mut revocation_unconfirmed = credential.is_err();
    if let Ok(Some((refresh, revoke))) = credential {
        let revoked = revoke_with_deadline(
            g.http
                .post(revoke)
                .form(&[
                    ("token", refresh.as_str()),
                    ("token_type_hint", "refresh_token"),
                ])
                .send(),
        )
        .await;
        if !revoked {
            revocation_unconfirmed = true;
        }
    }

    on_writer(&g, |w| {
        for name in SECRETS.into_iter().chain([ACCOUNT]) {
            delete_secret(w, name)?;
        }
        // The cursors belong to the account: a later connection lists everything again.
        for (_, _, name) in TASKS {
            let mut state = match writer::google::read_object(w.state(), name) {
                Ok(state) => state,
                Err(e) if unreadable_secret(&e) => {
                    delete_secret(w, name)?;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            if state.is_empty() {
                continue;
            }
            for cursor in ["history_id", "listed", "backfill_page", "window_end"] {
                if state.contains_key(cursor) {
                    state.insert(cursor.to_owned(), Value::Null);
                }
            }
            if state.contains_key("sync_tokens") {
                state.insert("sync_tokens".to_owned(), json!({}));
            }
            writer::google::write_object(w.state(), name, &state)?;
        }
        // The scopes belong to the connection.
        match writer::google::read_object(w.state(), SETTINGS) {
            Ok(mut s) => {
                if s.remove(SCOPES_KEY).is_some() {
                    writer::google::write_object(w.state(), SETTINGS, &s)?;
                }
            }
            // Unreadable settings cannot retain connection scopes or serve status reads.
            // Deletion needs no decryption key, just like the other credential cleanup.
            Err(e) if unreadable_secret(&e) => delete_secret(w, SETTINGS)?,
            Err(e) => return Err(e.into()),
        }
        Ok(())
    })
    .await?;
    let mut out = status_of(&g).await?;
    if revocation_unconfirmed {
        out["revocation_warning"] = json!(
            "Disconnected locally and paused Gmail and Calendar, but Google did not confirm token revocation. Revoke TheVault in your Google account security settings."
        );
    }
    Ok(Json(out))
}

/// Only unreadable ciphertext is disposable. Storage failures must remain errors.
fn unreadable_secret(e: &writer::Error) -> bool {
    matches!(
        e,
        writer::Error::State(
            state::Error::Corrupt(_)
                | state::Error::WrongKey { .. }
                | state::Error::KeyMissing { .. }
        )
    )
}

async fn revoke_with_deadline(
    request: impl std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
) -> bool {
    matches!(tokio::time::timeout(GOOGLE_TIMEOUT, request).await, Ok(Ok(res)) if res.status().is_success())
}

#[cfg(test)]
mod disconnect_tests {
    use super::*;

    #[test]
    fn queued_finish_cannot_reconnect_after_disconnect() {
        disconnect_race(false);
    }

    #[test]
    fn queued_finish_cannot_restore_deleted_settings() {
        disconnect_race(true);
    }

    fn disconnect_race(remove_settings: bool) {
        let dir = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        let state = state::State::open(
            &vault.server_dir().join("state.db"),
            Some(state::SecretKey::generate()),
        )
        .unwrap();
        let writer = Arc::new(Writer::open(vault, state).unwrap());
        let host = Arc::new(script::Host::new(
            writer.clone(),
            1,
            script::Limits::default(),
            script::Ai::of(&writer),
        ));
        let scheduler =
            Scheduler::new(writer.clone(), host, Arc::new(scheduler::TestClock::new(0))).unwrap();
        let google = Arc::new(Google::new(scheduler));
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let app = axum::Router::new()
                .route("/revoke", axum::routing::post({
                    let entered = entered.clone();
                    let release = release.clone();
                    move || async move {
                        entered.notify_one();
                        release.notified().await;
                        StatusCode::OK
                    }
                }))
                .route("/token", axum::routing::post(|| async {
                    Json(json!({"refresh_token": "new-refresh", "access_token": "access", "scope": SCOPES}))
                }))
                .route("/userinfo", axum::routing::get(|| async {
                    Json(json!({"email": "race@example.test"}))
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let provider = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let saved = json!({"endpoints": {
                "token": format!("{base}/token"),
                "userinfo": format!("{base}/userinfo"),
                "revoke": format!("{base}/revoke")
            }});
            for (name, value) in [(CLIENT_ID, "client"), (CLIENT_SECRET, "secret"), (REFRESH_TOKEN, "old-refresh")] {
                writer.state().set_secret(name, value).unwrap();
            }
            writer.state().set_secret(SETTINGS, &saved.to_string()).unwrap();
            google.pending().insert("race".into(), Pending {
                verifier: "verifier".into(), redirect_uri: LOOPBACK_REDIRECT.into(),
                endpoints: Endpoints::of(saved.as_object().unwrap()), started: Instant::now(),
            });
            let disconnect = disconnect(State(google.clone()));
            tokio::pin!(disconnect);
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    _ = entered.notified() => {},
                    result = &mut disconnect => panic!("disconnect ended before revoke: {result:?}"),
                }
            }).await.unwrap();
            // The endpoint is already captured. Model settings removed during the request.
            if remove_settings {
                delete_secret(&writer, SETTINGS).unwrap();
            }
            let finish = finish(State(google.clone()), Json(FinishBody {
                url: "http://127.0.0.1/?state=race&code=valid".into(),
            }));
            tokio::pin!(finish);
            // Poll the actual valid finish while revoke remains open, not an invalid-input probe.
            assert!(tokio::time::timeout(Duration::from_millis(100), &mut finish).await.is_err(),
                "finish escaped serialization while revoke was held");
            release.notify_one();
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut disconnect).await.unwrap().unwrap();
            let error = tokio::time::timeout(Duration::from_secs(5), &mut finish).await.unwrap().unwrap_err();
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
            for name in SECRETS.into_iter().chain([ACCOUNT]) {
                assert!(writer.state().get_secret(name).unwrap().is_none(), "{name}");
            }
            let settings = writer.state().get_secret(SETTINGS).unwrap();
            if remove_settings {
                assert!(settings.is_none());
            } else {
                assert_eq!(serde_json::from_str::<Value>(&settings.unwrap()).unwrap(), saved);
            }
            provider.abort();
        });
    }

    #[test]
    fn finish_and_disconnect_share_connection_exclusion() {
        let dir = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        let state = state::State::open(
            &vault.server_dir().join("state.db"),
            Some(state::SecretKey::generate()),
        )
        .unwrap();
        let writer = Arc::new(Writer::open(vault, state).unwrap());
        let host = Arc::new(script::Host::new(
            writer.clone(),
            1,
            script::Limits::default(),
            script::Ai::of(&writer),
        ));
        let scheduler =
            Scheduler::new(writer, host, Arc::new(scheduler::TestClock::new(0))).unwrap();
        let google = Arc::new(Google::new(scheduler));
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let guard = google.connection.lock().await;
            let finish = finish(
                State(google.clone()),
                Json(FinishBody {
                    url: "invalid".into(),
                }),
            );
            let disconnect = disconnect(State(google.clone()));
            tokio::pin!(finish, disconnect);
            // Neither route can even begin its work while the other owns the connection.
            assert!(futures_util::poll!(&mut finish).is_pending());
            assert!(futures_util::poll!(&mut disconnect).is_pending());
            drop(guard);
            assert!(finish.await.is_err());
        });
    }

    #[tokio::test(start_paused = true)]
    async fn revocation_has_a_bounded_total_deadline() {
        let began = tokio::time::Instant::now();
        assert!(!revoke_with_deadline(std::future::pending()).await);
        assert_eq!(began.elapsed(), GOOGLE_TIMEOUT);
    }

    #[test]
    fn only_ciphertext_errors_allow_deletion() {
        for e in [
            state::Error::Corrupt("test".into()),
            state::Error::WrongKey {
                detail: "test".into(),
            },
            state::Error::KeyMissing { path: None },
        ] {
            assert!(unreadable_secret(&e.into()));
        }
        assert!(!unreadable_secret(
            &state::Error::Io(std::io::Error::other("test")).into()
        ));
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBody {
    #[serde(default)]
    gmail_remote_images: Option<bool>,
    gmail_mode: String,
    gmail_labels: Vec<String>,
    gmail_window_days: i64,
    gmail_backfill_to: Option<String>,
    calendars: Vec<String>,
    default_calendar: String,
}

/// The longest Gmail window, in days: ten years.
const MAX_WINDOW_DAYS: i64 = 3650;

/// Every non-empty entry of `items`, trimmed, once each, in order.
fn cleaned(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let item = item.trim().to_owned();
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// `PUT /api/integrations/google/settings`.
pub async fn put_settings(
    State(g): Shared,
    crate::Caller(author): crate::Caller,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    if b.gmail_remote_images.is_some() && !matches!(author, state::Author::Session(_)) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "session_required",
            "Mail image consent requires an owner session",
        ));
    }
    if !matches!(b.gmail_mode.as_str(), "labels" | "all") {
        return Err(ApiError::bad_request(format!(
            "gmail_mode must be labels or all, not {:?}",
            b.gmail_mode
        )));
    }
    if !(1..=MAX_WINDOW_DAYS).contains(&b.gmail_window_days) {
        return Err(ApiError::bad_request(format!(
            "the Gmail window must be 1 to {MAX_WINDOW_DAYS} days, not {}",
            b.gmail_window_days
        )));
    }
    let backfill = b
        .gmail_backfill_to
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty());
    if let Some(d) = &backfill
        && !schema::is_date(d)
    {
        return Err(ApiError::bad_request(format!(
            "the back-fill date must be a date as YYYY-MM-DD, not {d:?}"
        )));
    }
    let calendars = cleaned(b.calendars);
    if calendars.is_empty() {
        return Err(ApiError::bad_request(
            "list at least one calendar, such as primary",
        ));
    }
    let default_calendar = match b.default_calendar.trim() {
        "" => calendars[0].clone(),
        c => c.to_owned(),
    };
    let labels = cleaned(b.gmail_labels);
    on_writer(&g, move |w| {
        let mut s = settings(w)?;
        if let Some(remote) = b.gmail_remote_images {
            s.insert("gmail_remote_images".into(), json!(remote));
        }
        s.insert("gmail_mode".into(), json!(b.gmail_mode));
        s.insert("gmail_labels".into(), json!(labels));
        s.insert("gmail_window_days".into(), json!(b.gmail_window_days));
        s.insert("gmail_backfill_to".into(), json!(backfill));
        s.insert("calendars".into(), json!(calendars));
        s.insert("default_calendar".into(), json!(default_calendar));
        writer::google::write_object(w.state(), SETTINGS, &s)?;
        Ok(())
    })
    .await?;
    status_of(&g).await.map(Json)
}

/// `GET /api/integrations/google`.
pub async fn status(State(g): Shared) -> Result<Json<Value>, ApiError> {
    status_of(&g).await.map(Json)
}

async fn status_of(g: &Arc<Google>) -> Result<Value, ApiError> {
    let s = g.scheduler.clone();
    tokio::task::spawn_blocking(move || {
        let w = s.writer();
        let names: Vec<String> = w
            .state()
            .list_secrets()?
            .into_iter()
            .map(|s| s.name)
            .collect();
        let has = |n: &str| names.iter().any(|x| x == n);
        let account = if has(ACCOUNT) && w.state().has_secret_key() {
            w.state().get_secret(ACCOUNT)?
        } else {
            None
        };
        let mut settings = settings(w)?;
        // The test seam is the server's own business.
        settings.remove("endpoints");
        let connected = has(CLIENT_ID) && has(CLIENT_SECRET) && has(REFRESH_TOKEN);
        let granted = settings
            .remove(SCOPES_KEY)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let grants = |scope: &str| connected && granted.iter().any(|v| v.as_str() == Some(scope));
        let mut out = json!({
            "connected": connected,
            "client_stored": has(CLIENT_ID) && has(CLIENT_SECRET),
            "can_send": grants(SEND_SCOPE),
            "can_modify": grants(MODIFY_SCOPE),
            "account": account,
            "settings": settings,
        });
        for (task, side, secret) in TASKS {
            let (installed, paused, running, summary) = match s.status(task) {
                Ok(st) => (
                    true,
                    st.def.as_ref().ok().map(|d| d.paused),
                    st.running,
                    st.latest_run.as_ref().and_then(summary_of),
                ),
                Err(scheduler::Error::NoTask(_)) => (false, None, false, None),
                Err(e) => return Err(ApiError::from(e)),
            };
            let state = object(w, secret)?;
            let text = |k: &str| state.get(k).and_then(Value::as_str).map(str::to_owned);
            out[side] = json!({
                "task": task,
                "installed": installed,
                "paused": paused,
                "running": running,
                "last_run": text("last_run").map(|t| utc(&t)),
                "last_error": text("last_error"),
                "summary": summary,
            });
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

/// The last line a finished run logged, such as `Gmail: 3 new, 1 relabelled, 0 archived, 0 not
/// saved.`: the sync scripts end on their summary. `None` while it runs or when it logged none.
fn summary_of(run: &state::TaskRun) -> Option<String> {
    run.ended_ms?;
    run.log
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_owned)
}

/// An RFC 3339 time in UTC to the second, ending `Z`, whatever offset it was written with; the
/// Gmail script writes `+00:00` and the Calendar script `Z`. Text that is not a time stays as it
/// is.
fn utc(text: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(text) {
        Ok(t) => t
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        Err(_) => text.to_owned(),
    }
}
