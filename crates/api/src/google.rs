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
//!   `google_sync` settings note's `account` (making the note when there is none), unpauses the
//!   `google_gmail` and `google_calendar` tasks that exist, and runs each once in the
//!   background. An error from Google is a 400 carrying Google's message.
//! - `POST /api/integrations/google/disconnect` deletes the three secrets, pauses the tasks and
//!   clears `account`.
//! - `GET /api/integrations/google` answers `{ connected, account, client_stored, gmail,
//!   calendar }`, each side `{ installed, paused, last_run, last_error }`, from the settings note,
//!   the tasks and which secrets exist. `paused` is `null` when the task does not exist.
//!
//! The token and userinfo endpoints come from the settings note's hidden `endpoints` dict when it
//! names them, else Google's own (the test seam). A flow keeps the endpoints it started with, so
//! a settings note made by `finish` carries them on.

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
use schema::Value as Field;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use vocab::Author;
use writer::{Input, Writer};

use crate::ApiError;

/// The settings note's type.
const SETTINGS_TYPE: &str = "google_sync";

/// The secrets the sign-in stores and the sync scripts read (google-sync D19).
pub const CLIENT_ID: &str = "google_client_id";
pub const CLIENT_SECRET: &str = "google_client_secret";
pub const REFRESH_TOKEN: &str = "google_refresh_token";
const SECRETS: [&str; 3] = [CLIENT_ID, CLIENT_SECRET, REFRESH_TOKEN];

/// The tasks a connection unpauses and a disconnection pauses, with the prefix of their fields
/// on the settings note.
const TASKS: [(&str, &str); 2] = [("google_gmail", "gmail"), ("google_calendar", "calendar")];

/// The scopes the consent page asks for.
pub const SCOPES: &str = "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/calendar openid email";

/// Google's consent page.
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google's token endpoint.
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Google's userinfo endpoint.
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";

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
}

impl Google {
    pub fn new(scheduler: Arc<Scheduler>) -> Google {
        Google {
            scheduler,
            pending: Mutex::default(),
            http: reqwest::Client::builder()
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
    dict: Option<Map<String, Value>>,
}

impl Endpoints {
    fn of(note: Option<&writer::Note>) -> Endpoints {
        let dict = note.and_then(|n| {
            n.doc
                .blocks
                .iter()
                .find(|b| b.kind == "dict" && b.label() == Some("endpoints"))
                .map(|b| {
                    b.body
                        .fields
                        .iter()
                        .map(|(k, v)| (k.clone(), writer::value_to_json(v)))
                        .collect::<Map<String, Value>>()
                })
        });
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

/// The settings note: the first `google_sync` note, when there is one.
fn settings(w: &Writer) -> Result<Option<writer::Note>, ApiError> {
    if w.schema().type_def(SETTINGS_TYPE).is_none() {
        return Ok(None);
    }
    match w.list(Some(SETTINGS_TYPE))?.into_iter().next() {
        Some(s) => Ok(Some(w.read(&s.id)?)),
        None => Ok(None),
    }
}

fn author() -> Author {
    Author::Task(writer::SERVER_AUTHOR.to_owned())
}

/// Set the settings note's `account`, or remove it for `None`. Makes the note, with
/// `endpoints`, when there is none and `account` is set.
fn write_account(
    w: &Writer,
    account: Option<&str>,
    endpoints: Option<&Map<String, Value>>,
) -> Result<(), ApiError> {
    match settings(w)? {
        Some(note) => {
            let mut doc = note.doc.clone();
            doc.fields.retain(|(n, _)| n != "account");
            if let Some(a) = account {
                doc.fields
                    .push(("account".to_owned(), Field::Str(a.to_owned())));
            }
            if doc == note.doc {
                return Ok(());
            }
            let input = Input::Json(writer::doc_to_json(&doc));
            w.update(&note.id, &input, Some(&note.version), &author())?;
        }
        None => {
            let Some(a) = account else { return Ok(()) };
            let mut blocks = Vec::new();
            if let Some(d) = endpoints {
                blocks.push(json!({ "kind": "dict", "labels": ["endpoints"], "fields": d }));
            }
            let input = Input::Json(json!({
                "fields": { "title": "Google", "account": a },
                "blocks": blocks,
            }));
            w.create(SETTINGS_TYPE, &input, &author())?;
        }
    }
    Ok(())
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
        Ok(Endpoints::of(settings(w)?.as_ref()))
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
        write_account(w, Some(&account), dict.as_ref())
    })
    .await?;
    let s = g.scheduler.clone();
    let existing = tokio::task::spawn_blocking(move || {
        let mut existing = Vec::new();
        for (task, _) in TASKS {
            if s.set_paused(task, false)? {
                existing.push(task);
            }
        }
        Ok::<_, scheduler::Error>(existing)
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))??;
    for task in existing {
        let s = g.scheduler.clone();
        tokio::task::spawn_blocking(move || {
            // A run already going is as good as this one.
            let _ = s.trigger(task);
        });
    }
    status_of(&g).await.map(Json)
}

/// `POST /api/integrations/google/disconnect`.
pub async fn disconnect(State(g): Shared) -> Result<Json<Value>, ApiError> {
    on_writer(&g, |w| {
        for name in SECRETS {
            match w.state().delete_secret(name) {
                Ok(()) | Err(state::Error::UnknownSecret(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        write_account(w, None, None)
    })
    .await?;
    let s = g.scheduler.clone();
    tokio::task::spawn_blocking(move || {
        for (task, _) in TASKS {
            s.set_paused(task, true)?;
        }
        Ok::<_, scheduler::Error>(())
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))??;
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
        let note = settings(w)?;
        let field = |name: &str| {
            note.as_ref()
                .and_then(|n| n.doc.field(name))
                .and_then(Field::as_str)
                .map(str::to_owned)
        };
        let mut out = json!({
            "connected": has(CLIENT_ID) && has(CLIENT_SECRET) && has(REFRESH_TOKEN),
            "client_stored": has(CLIENT_ID) && has(CLIENT_SECRET),
            "account": field("account"),
            "settings": note.as_ref().map(|n| n.id.clone()),
        });
        for (task, side) in TASKS {
            let (installed, paused) = match s.status(task) {
                Ok(st) => (true, st.def.as_ref().ok().map(|d| d.paused)),
                Err(scheduler::Error::NoTask(_)) => (false, None),
                Err(e) => return Err(ApiError::from(e)),
            };
            out[side] = json!({
                "task": task,
                "installed": installed,
                "paused": paused,
                "last_run": field(&format!("last_{side}_run")),
                "last_error": field(&format!("last_{side}_error")),
            });
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}
