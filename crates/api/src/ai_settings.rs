//! Settings > AI: the provider keys, Sign in with ChatGPT, the model lists and the Test button
//! (ai D6, D7, D8, D15, D16). The calls themselves are `crates/ai`'s.
//!
//! - `GET /api/integrations/ai` answers `{ secrets_key, default_provider, providers: { anthropic:
//!   { configured, model, url, has_key }, openai: { configured, model, url, has_key }, chatgpt: {
//!   configured, account, model } } }, system_prompt, default_system_prompt, system_prompt_custom,
//!   confirm }`, the OpenAI entry also with `api`, `responses` or `chat_completions`, the API its
//!   calls speak (always `responses` without a custom URL, ai D25): each API URL as stored, or
//!   null for the official API, and whether a key is
//!   stored, which a custom URL makes optional (ai D24), the chat system prompt in
//!   use, the shipped one, and whether they differ (ai D17), and `confirm: { notes, archive,
//!   delete, types }` with every key filled (ai D22). A provider with a custom API URL counts as
//!   configured without a key. No answer of these routes ever holds a key or a token.
//! - `PUT /api/integrations/ai/settings` takes `{ default_provider?, models?: { anthropic?,
//!   openai?, chatgpt? }, system_prompt?, confirm?: { notes?, archive?, delete?, types? }, urls?:
//!   { anthropic?, openai? }, openai_api? }`; a field sent replaces the stored one, and an empty
//!   string clears it, which for `system_prompt` means the default, for a URL the official API
//!   and for `openai_api` (`responses` or `chat_completions`, else a 400) Responses. A system
//!   prompt over [`ai::SYSTEM_PROMPT_MAX`] characters is a 400; a URL [`ai::check_url`] refuses
//!   is a 400 `invalid_url`. The rest of `endpoints`, the test seam, is kept.
//! - `PUT /api/integrations/ai/{anthropic|openai}/key` takes `{ key }` and stores it in
//!   `ai_<provider>_api_key`; an empty key deletes the secret.
//! - `POST /api/integrations/ai/chatgpt/start` builds the authorize URL of ai D8 and holds the
//!   PKCE verifier and nonce for [`PENDING_FOR`] under its `state`. The first sign-in sends
//!   `client_id=dynamic_agent_client` with `agent_name_hint`; later ones send the issued client
//!   id with `id_token_hint` and `login_hint`. The host id is made once and kept.
//! - `POST /api/integrations/ai/chatgpt/finish` takes `{ url }`, the pasted callback address. It
//!   checks the state, exchanges the code for the issued client id, verifies the ID token against
//!   the issuer's JWKS, requires the plan scope, and stores `ai_chatgpt`. Every refusal is a 400.
//! - `POST /api/integrations/ai/chatgpt/disconnect` revokes the refresh token (best effort) and
//!   deletes `ai_chatgpt`.
//! - `POST /api/integrations/ai/{provider}/test` takes an optional `{ model }` and sends the
//!   16-token "Reply with OK" request to it, else to the provider's default model, else to the
//!   model [`ai::preselect`] picks from the live list, which it then saves as the default. On
//!   OpenAI with a custom API URL it detects the API (ai D25): when the saved one's path answers
//!   404 or 405, it sends the same request on the other API's path, and when that path answers,
//!   saves the other as `openai_api`. It answers 200 `{ ok, provider, model, ms, text, api,
//!   error: null | { kind, message } }`, `api` being the OpenAI API the answer came over, or null
//!   for the other providers and the official OpenAI API.
//! - `GET /api/ai/providers` (scope `read`) answers `{ default_provider, providers: { <p>: {
//!   configured, model } }, confirm }`, the chat panel's picker in every client (ai D23): no
//!   account, no prompt, no API URL, never a secret.
//! - `GET /api/ai/models?provider=` (scope `read`) lists the provider's models live, with the
//!   default model, or the preselected one when none is chosen.
//!
//! Credential resolution and the sign-in calls block, so they run in `spawn_blocking`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ai::{AiError, ChatGptAuth, Credentials, ErrorKind, Provider, Settings, siwc};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use writer::Writer;

use crate::ApiError;

/// How long a started sign-in waits for its `finish`.
const PENDING_FOR: Duration = Duration::from_secs(10 * 60);

/// How long the Test request may take.
const TEST_TIMEOUT: Duration = Duration::from_secs(90);

/// The routes' state: the writer, whose state store holds the secrets, and the sign-ins started.
pub struct Ai {
    pub(crate) writer: Arc<Writer>,
    pending: Mutex<HashMap<String, Pending>>,
    http: reqwest::Client,
    /// The chat turns started through `POST /api/ai/chat/turns` (ai D23).
    pub(crate) turns: crate::ai_turns::Turns,
}

impl Ai {
    pub fn new(writer: Arc<Writer>) -> Ai {
        Ai {
            writer,
            pending: Mutex::default(),
            http: ai::client(),
            turns: crate::ai_turns::Turns::default(),
        }
    }

    /// The started sign-ins, without those older than [`PENDING_FOR`].
    fn pending(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, v| v.started.elapsed() < PENDING_FOR);
        p
    }
}

type Shared = State<Arc<Ai>>;

/// A ChatGPT sign-in waiting for its `finish`.
struct Pending {
    verifier: String,
    nonce: String,
    /// The sign-in server the start used.
    auth_base: String,
    started: Instant,
}

/// Run blocking work against the state store off the async runtime.
async fn blocking<T: Send + 'static>(
    a: &Arc<Ai>,
    f: impl FnOnce(&state::State) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let w = a.writer.clone();
    tokio::task::spawn_blocking(move || f(w.state()))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
}

/// An AI error as an answer: its kind is the error code. A missing credential or a bad request
/// is the caller's to fix (400); anything the provider did is a 502.
fn api_error(e: AiError) -> ApiError {
    let status = match e.kind {
        ErrorKind::NotConfigured | ErrorKind::Invalid => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    };
    ApiError::new(status, e.kind.as_str(), e.message)
}

/// A refused API URL (ai D24): a 400 `invalid_url`.
fn url_error(e: AiError) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_url", e.message)
}

/// A refused ChatGPT sign-in step: always a 400 the card shows as it is.
fn chatgpt_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "chatgpt", message)
}

fn provider(name: &str) -> Result<Provider, ApiError> {
    name.parse::<Provider>().map_err(ApiError::bad_request)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn delete_secret(s: &state::State, name: &str) -> Result<(), ApiError> {
    match s.delete_secret(name) {
        Ok(()) | Err(state::Error::UnknownSecret(_)) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Make `p` the default provider when none is chosen yet.
fn default_to(s: &state::State, p: Provider) -> Result<(), ApiError> {
    let mut settings = Settings::load(s).map_err(api_error)?;
    if settings.default_provider.is_none() {
        settings.default_provider = Some(p);
        settings.save(s).map_err(api_error)?;
    }
    Ok(())
}

/// What `GET /api/integrations/ai` answers. Nothing in it is a key or a token.
fn status_of(s: &state::State) -> Result<Value, ApiError> {
    if !s.has_secret_key() {
        let off = json!({ "configured": false, "model": null, "url": null, "has_key": false });
        return Ok(json!({
            "secrets_key": false,
            "default_provider": null,
            "providers": {
                "anthropic": off,
                "openai": off,
                "chatgpt": { "configured": false, "account": null, "model": null },
            },
            "system_prompt": ai::CHAT_PROMPT,
            "default_system_prompt": ai::CHAT_PROMPT,
            "system_prompt_custom": false,
            "confirm": ai::Confirm::default().to_json(),
        }));
    }
    let settings = Settings::load(s).map_err(api_error)?;
    let mut providers = Map::new();
    for p in Provider::ALL {
        let configured = Credentials::configured(s, p).map_err(api_error)?;
        let mut entry = json!({ "configured": configured, "model": settings.model(p) });
        if p == Provider::ChatGpt {
            let auth = ChatGptAuth::load(s).map_err(api_error)?;
            entry["account"] = json!(auth.account.filter(|_| configured));
        } else {
            entry["url"] = json!(settings.endpoints.url(p));
            if p == Provider::OpenAi {
                entry["api"] = json!(settings.endpoints.openai_api().as_str());
            }
            let key = s.get_secret(ai::key_secret(p))?;
            entry["has_key"] = json!(key.is_some_and(|k| !k.trim().is_empty()));
        }
        providers.insert(p.as_str().to_owned(), entry);
    }
    Ok(json!({
        "secrets_key": true,
        "default_provider": settings.default_provider,
        "providers": providers,
        "system_prompt": settings.chat_prompt(),
        "default_system_prompt": ai::CHAT_PROMPT,
        "system_prompt_custom": settings.system_prompt_custom(),
        "confirm": settings.confirm.to_json(),
    }))
}

async fn status_answer(a: &Arc<Ai>) -> Result<Json<Value>, ApiError> {
    blocking(a, status_of).await.map(Json)
}

/// `GET /api/ai/providers` (read): which providers are set up and their default models, the
/// default provider and the confirmation switches, for the chat panel's picker in every client
/// (ai D23). Unlike `GET /api/integrations/ai` it holds no account and no system prompt.
pub async fn providers(State(a): Shared) -> Result<Json<Value>, ApiError> {
    let status = blocking(&a, status_of).await?;
    let mut providers = Map::new();
    if let Some(all) = status["providers"].as_object() {
        for (name, p) in all {
            providers.insert(
                name.clone(),
                json!({ "configured": p["configured"], "model": p["model"] }),
            );
        }
    }
    Ok(Json(json!({
        "default_provider": status["default_provider"],
        "providers": providers,
        "confirm": status["confirm"],
    })))
}

/// `GET /api/integrations/ai`.
pub async fn status(State(a): Shared) -> Result<Json<Value>, ApiError> {
    status_answer(&a).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBody {
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default)]
    models: Option<HashMap<String, Option<String>>>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    confirm: Option<ConfirmBody>,
    #[serde(default)]
    urls: Option<HashMap<String, Option<String>>>,
    #[serde(default)]
    openai_api: Option<String>,
}

/// A partial `confirm`: each switch sent replaces the stored one (ai D22).
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfirmBody {
    #[serde(default)]
    notes: Option<bool>,
    #[serde(default)]
    archive: Option<bool>,
    #[serde(default)]
    delete: Option<bool>,
    #[serde(default)]
    types: Option<bool>,
}

/// `PUT /api/integrations/ai/settings`.
pub async fn put_settings(
    State(a): Shared,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    let default = match b.default_provider.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(p) => Some(Some(provider(p)?)),
    };
    let mut models = Vec::new();
    for (name, model) in b.models.unwrap_or_default() {
        models.push((provider(&name)?, model.map(|m| m.trim().to_owned())));
    }
    let mut urls = Vec::new();
    for (name, url) in b.urls.unwrap_or_default() {
        let p = provider(&name)?;
        // Checked before anything is saved, so a bad URL changes nothing.
        let url = match url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => Some(ai::check_url(u).map_err(url_error)?),
            None => None,
        };
        urls.push((p, url));
    }
    let openai_api = match b.openai_api.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(a) => Some(Some(
            a.parse::<ai::OpenAiApi>().map_err(ApiError::bad_request)?,
        )),
    };
    let prompt = b.system_prompt;
    let confirm = b.confirm;
    blocking(&a, move |s| {
        let mut settings = Settings::load(s).map_err(api_error)?;
        for (p, url) in urls {
            settings
                .endpoints
                .set_url(p, url.as_deref())
                .map_err(url_error)?;
        }
        if let Some(api) = openai_api {
            settings.endpoints.openai_api = api;
        }
        if let Some(c) = confirm {
            let to = &mut settings.confirm;
            for (slot, value) in [
                (&mut to.notes, c.notes),
                (&mut to.archive, c.archive),
                (&mut to.delete, c.delete),
                (&mut to.types, c.types),
            ] {
                if let Some(v) = value {
                    *slot = v;
                }
            }
        }
        if let Some(p) = prompt {
            settings.set_system_prompt(&p).map_err(api_error)?;
        }
        if let Some(d) = default {
            settings.default_provider = d;
        }
        for (p, m) in models {
            settings.set_model(p, m);
        }
        settings.save(s).map_err(api_error)?;
        status_of(s)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBody {
    key: String,
}

/// `PUT /api/integrations/ai/{provider}/key`.
pub async fn put_key(
    State(a): Shared,
    Path(name): Path<String>,
    Json(b): Json<KeyBody>,
) -> Result<Json<Value>, ApiError> {
    let p = provider(&name)?;
    if p == Provider::ChatGpt {
        return Err(ApiError::bad_request(
            "the ChatGPT plan takes no key: Continue with ChatGPT instead",
        ));
    }
    let key = b.key.trim().to_owned();
    blocking(&a, move |s| {
        let secret = ai::key_secret(p);
        if key.is_empty() {
            delete_secret(s, secret)?;
        } else {
            s.set_secret(secret, &key)?;
            default_to(s, p)?;
        }
        status_of(s)
    })
    .await
    .map(Json)
}

/// `POST /api/integrations/ai/chatgpt/start`.
pub async fn chatgpt_start(State(a): Shared) -> Result<Json<Value>, ApiError> {
    let (auth, auth_base) = blocking(&a, |s| {
        let mut auth = ChatGptAuth::load(s).map_err(api_error)?;
        if auth.host_id.as_deref().is_none_or(str::is_empty) {
            auth.host_id = Some(siwc::new_host_id());
            auth.save(s).map_err(api_error)?;
        }
        let settings = Settings::load(s).map_err(api_error)?;
        Ok((auth, settings.endpoints.chatgpt_auth_base()))
    })
    .await?;
    let (verifier, challenge) = siwc::pkce();
    let state = siwc::random_token();
    let nonce = siwc::random_token();
    let issued = auth.client_id.as_deref().filter(|c| !c.is_empty());
    let auth_url = siwc::authorize_url(
        &auth_base,
        &siwc::Authorize {
            client_id: issued,
            host_id: auth.host_id.as_deref().unwrap_or_default(),
            state: &state,
            nonce: &nonce,
            code_challenge: &challenge,
            id_token_hint: issued
                .and(auth.id_token.as_ref())
                .map(|t| t.expose())
                .filter(|t| !t.is_empty()),
            login_hint: issued.and(auth.account.as_deref()),
        },
    );
    a.pending().insert(
        state,
        Pending {
            verifier,
            nonce,
            auth_base,
            started: Instant::now(),
        },
    );
    Ok(Json(json!({ "auth_url": auth_url })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishBody {
    url: String,
}

/// `POST /api/integrations/ai/chatgpt/finish`.
pub async fn chatgpt_finish(
    State(a): Shared,
    Json(b): Json<FinishBody>,
) -> Result<Json<Value>, ApiError> {
    let url = Url::parse(b.url.trim()).map_err(|_| {
        chatgpt_error(
            "paste the whole address from the browser's address bar, starting http://127.0.0.1:53683",
        )
    })?;
    let param = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    let pending = param("state").and_then(|s| a.pending().remove(&s));
    if let Some(error) = param("error") {
        let why =
            param("error_description").unwrap_or_else(|| "the sign-in was not approved".into());
        return Err(chatgpt_error(format!(
            "ChatGPT answered {error}: {why}. Continue with ChatGPT again."
        )));
    }
    let Some(pending) = pending else {
        return Err(chatgpt_error(
            "this sign-in was not started here, or it expired or was already used: Continue with ChatGPT again",
        ));
    };
    let code = param("code")
        .ok_or_else(|| chatgpt_error("the address has no code: Continue with ChatGPT again"))?;
    let issued = param("client_id");

    blocking(&a, move |s| {
        let mut auth = ChatGptAuth::load(s).map_err(api_error)?;
        let Some(client_id) = issued.or_else(|| auth.client_id.clone().filter(|c| !c.is_empty()))
        else {
            return Err(chatgpt_error(
                "the address names no client_id: paste the whole address and try again",
            ));
        };
        let http = ai::blocking_client();
        let refused = |e: AiError| chatgpt_error(e.message);
        let tokens = siwc::exchange_code(
            &http,
            &pending.auth_base,
            &client_id,
            &code,
            &pending.verifier,
        )
        .map_err(refused)?;
        let config = siwc::openid_configuration(&http, &pending.auth_base).map_err(refused)?;
        // Tokens that cannot be used are revoked before the refusal, best effort.
        let discard = |why: ApiError| {
            if let (Some(r), Some(end)) = (&tokens.refresh_token, &config.revocation_endpoint) {
                let _ = siwc::revoke(&http, end, &client_id, r);
            }
            why
        };
        let Some(id_token) = tokens.id_token.clone() else {
            return Err(discard(chatgpt_error(
                "the ChatGPT sign-in server sent no ID token: Continue with ChatGPT again",
            )));
        };
        let keys = siwc::jwks(&http, &config.jwks_uri).map_err(refused)?;
        let claims = siwc::verify_id_token(
            id_token.expose(),
            &keys,
            &config.issuer,
            &client_id,
            &pending.nonce,
            now_ms(),
        )
        .map_err(|e| discard(refused(e)))?;
        if !tokens.has_plan_scope() {
            return Err(discard(chatgpt_error(format!(
                "ChatGPT signed in but did not grant {}, so TheVault cannot use your plan. \
                 Continue with ChatGPT again and allow access to your plan.",
                siwc::PLAN_SCOPE
            ))));
        }
        if tokens.refresh_token.is_none() {
            return Err(chatgpt_error(
                "the ChatGPT sign-in server sent no refresh token: Continue with ChatGPT again",
            ));
        }
        auth.client_id = Some(client_id.clone());
        auth.store_tokens(tokens, now_ms());
        auth.account = claims.email.or(claims.subject);
        auth.save(s).map_err(api_error)?;
        default_to(s, Provider::ChatGpt)?;
        status_of(s)
    })
    .await
    .map(Json)
}

/// `POST /api/integrations/ai/chatgpt/disconnect`.
pub async fn chatgpt_disconnect(State(a): Shared) -> Result<Json<Value>, ApiError> {
    blocking(&a, |s| {
        let auth = ChatGptAuth::load(s).map_err(api_error)?;
        if let (Some(client_id), Some(refresh)) = (&auth.client_id, &auth.refresh_token) {
            let base = Settings::load(s)
                .map_err(api_error)?
                .endpoints
                .chatgpt_auth_base();
            let http = ai::blocking_client();
            let revoked = siwc::openid_configuration(&http, &base).and_then(|c| {
                match c.revocation_endpoint {
                    Some(end) => siwc::revoke(&http, &end, client_id, refresh),
                    None => Err(AiError::invalid(
                        "the OpenID configuration names no revocation endpoint",
                    )),
                }
            });
            if let Err(e) = revoked {
                eprintln!("ai: revoking the ChatGPT sign-in: {}", e.message);
            }
        }
        delete_secret(s, ai::CHATGPT)?;
        status_of(s)
    })
    .await
    .map(Json)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TestBody {
    #[serde(default)]
    model: Option<String>,
}

/// The credential for `p`, and the model a Test or a listing uses: the one asked for, else the
/// default.
async fn credential(
    a: &Arc<Ai>,
    p: Provider,
) -> Result<Result<(ai::Credential, Settings), AiError>, ApiError> {
    blocking(a, move |s| {
        Ok(Credentials::resolve(s, p).and_then(|c| Settings::load(s).map(|st| (c, st))))
    })
    .await
}

/// `POST /api/integrations/ai/{provider}/test`.
pub async fn test(
    State(a): Shared,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let p = provider(&name)?;
    let b: TestBody = if body.iter().all(u8::is_ascii_whitespace) {
        TestBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?
    };
    let began = Instant::now();
    let answer_with =
        move |model: Option<&str>, result: Result<String, AiError>, api: Option<ai::OpenAiApi>| {
            let ms = began.elapsed().as_millis() as u64;
            let api = api.map(ai::OpenAiApi::as_str);
            match result {
                Ok(text) => json!({
                    "ok": true, "provider": p, "model": model, "ms": ms, "text": text, "api": api,
                    "error": null,
                }),
                Err(e) => json!({
                    "ok": false, "provider": p, "model": model, "ms": ms, "text": null, "api": api,
                    "error": { "kind": e.kind.as_str(), "message": e.message },
                }),
            }
        };
    let answer =
        |model: Option<&str>, result: Result<String, AiError>| answer_with(model, result, None);
    let (cred, settings) = match credential(&a, p).await? {
        Ok(c) => c,
        Err(e) => return Ok(Json(answer(None, Err(e)))),
    };
    let asked = b
        .model
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty());
    let model = match asked.or_else(|| settings.model(p).map(str::to_owned)) {
        Some(m) => m,
        None => {
            let models = match ai::list_models(&a.http, &cred).await {
                Ok(m) => m,
                Err(e) => return Ok(Json(answer(None, Err(e)))),
            };
            let Some(m) = ai::preselect(p, &models).map(|m| m.id.clone()) else {
                let e = AiError::not_configured(format!(
                    "no {} model is chosen: pick one from the list",
                    p.label()
                ));
                return Ok(Json(answer(None, Err(e))));
            };
            let keep = m.clone();
            blocking(&a, move |s| {
                let mut settings = Settings::load(s).map_err(api_error)?;
                if settings.model(p).is_none() {
                    settings.set_model(p, Some(keep));
                    settings.save(s).map_err(api_error)?;
                }
                Ok(())
            })
            .await?;
            m
        }
    };
    let req = ai::Request::test(p, model.clone());
    let mut result = test_call(&a, &cred, &req).await;
    // Only OpenAI with a custom API URL has a choice of API (ai D25).
    let mut api = (p == Provider::OpenAi && cred.custom_host.is_some()).then(|| cred.api());
    if let Some(saved) = api
        && result.as_ref().is_err_and(AiError::is_unsupported_api)
    {
        let mut other = cred.clone();
        other.openai_api = saved.other();
        let second = test_call(&a, &other, &req).await;
        // The other path answered, even with an error of its own: the service speaks it.
        if !second.as_ref().is_err_and(AiError::is_unsupported_api) {
            let found = saved.other();
            blocking(&a, move |s| {
                let mut settings = Settings::load(s).map_err(api_error)?;
                settings.endpoints.openai_api = Some(found);
                settings.save(s).map_err(api_error)
            })
            .await?;
            api = Some(found);
            result = second;
        }
    }
    Ok(Json(answer_with(Some(&model), result.map(|r| r.text), api)))
}

/// One Test request, under [`TEST_TIMEOUT`], leaving its trail line.
async fn test_call(
    a: &Arc<Ai>,
    cred: &ai::Credential,
    req: &ai::Request,
) -> Result<ai::Reply, AiError> {
    let p = req.provider;
    let began = Instant::now();
    let result = match tokio::time::timeout(TEST_TIMEOUT, ai::complete(&a.http, cred, req)).await {
        Ok(r) => r,
        Err(_) => Err(AiError::new(
            ErrorKind::Timeout,
            format!(
                "{} did not answer within {} seconds",
                p.label(),
                TEST_TIMEOUT.as_secs()
            ),
        )),
    };
    let host = cred.custom_host.as_deref();
    eprintln!(
        "{}",
        ai::trail_for(p, &req.model, &result, began.elapsed(), host)
    );
    result
}

#[derive(Deserialize)]
pub struct ModelsQuery {
    #[serde(default)]
    provider: Option<String>,
}

/// `GET /api/ai/models?provider=`.
pub async fn models(
    State(a): Shared,
    Query(q): Query<ModelsQuery>,
) -> Result<Json<Value>, ApiError> {
    let p = match q
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(name) => provider(name)?,
        None => {
            blocking(&a, |s| {
                Settings::load(s)
                    .map_err(api_error)?
                    .default_provider
                    .ok_or_else(|| {
                        api_error(AiError::not_configured(
                            "no AI provider is chosen: pick one in Settings > AI",
                        ))
                    })
            })
            .await?
        }
    };
    let (cred, settings) = credential(&a, p).await?.map_err(api_error)?;
    let list = ai::list_models(&a.http, &cred).await.map_err(api_error)?;
    let model = settings
        .model(p)
        .map(str::to_owned)
        .or_else(|| ai::preselect(p, &list).map(|m| m.id.clone()));
    Ok(Json(
        json!({ "provider": p, "model": model, "models": list }),
    ))
}
