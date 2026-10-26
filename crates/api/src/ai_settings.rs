//! Settings > AI: the table of named AI accounts, their keys, Sign in with ChatGPT per account,
//! the model lists and the Test button (ai D6, D7, D8, D15, D16; accounts A1 to A7, A13). The
//! calls themselves are `crates/ai`'s.
//!
//! - `GET /api/integrations/ai` answers `{ secrets_key, accounts: [AccountStatus], default,
//!   kinds, context: { auto_compact, compact_at }, confirm, web, fetch_private, system_prompt,
//!   default_system_prompt, system_prompt_custom }`, `kinds` being `[{ kind, label,
//!   default_url, default_window }]` for the four kinds in the order the Add form offers them. An `AccountStatus` is the account record
//!   (`id, name, kind, base_url, model, headers, context`) with `kind_label`, `default_base_url`
//!   (null for `openai_compatible`), `default_window`, `window` (the one in use), `configured`,
//!   `key_set`, `signed_in` (`{ account, expired }` for a `chatgpt` account that has signed in,
//!   else null), `agents` (the agents whose `account` names it) and `usage` (the plan's last
//!   usage reading, accounts A13, or null). `confirm` and `web` have every key filled (ai D22,
//!   D28). No answer of these routes ever holds a key or a token.
//! - `POST /api/integrations/ai/accounts` takes `{ name, kind, base_url?, model?, headers?,
//!   context?: { window } | null, key? }` and answers the new `AccountStatus`. `PUT
//!   .../accounts/{id}` takes the same fields but `kind` and `key`, each sent replacing the
//!   stored one (null or an empty string clearing it); a rename rewrites every agent file whose
//!   `account` names the old name, in one commit (accounts A6). `DELETE .../accounts/{id}`
//!   deletes the account with its secrets; agents naming it fall back to the default. `PUT
//!   .../accounts/order` takes `{ ids }`, every account once. `PUT .../default` takes `{ id }`,
//!   null or empty clearing it. A refusal is a 400; a bad URL is `invalid_url`.
//! - `PUT .../accounts/{id}/key` takes `{ key }`; an empty key deletes it. A `chatgpt` account
//!   takes none.
//! - `POST .../accounts/{id}/chatgpt/start` builds the authorize URL of ai D8 and holds the PKCE
//!   verifier and nonce for [`PENDING_FOR`] under its `state`, for that account. The first
//!   sign-in sends `client_id=dynamic_agent_client` with `agent_name_hint`; later ones send the
//!   issued client id with `id_token_hint` and `login_hint`. The host id is made once and kept.
//! - `POST .../accounts/{id}/chatgpt/finish` takes `{ url }`, the pasted callback address. It
//!   checks the state, exchanges the code for the issued client id, verifies the ID token
//!   against the issuer's JWKS, requires the plan scope, and stores the account's sign-in. Every
//!   refusal is a 400.
//! - `POST .../accounts/{id}/chatgpt/disconnect` revokes the refresh token (best effort) and
//!   deletes the account's sign-in.
//! - `POST .../accounts/{id}/test` takes an optional `{ model }` and sends the 16-token "Reply
//!   with OK" request to it, else to the account's model, else to the model [`ai::preselect`]
//!   picks from the live list, which it then saves as the account's. It answers 200 `{ ok,
//!   account, name, kind, model, ms, text, error: null | { kind, message } }`. A service that
//!   lacks the kind's API answers the `invalid` "This service does not support ..." error.
//! - `PUT /api/integrations/ai/settings` takes `{ system_prompt?, confirm?, web?, fetch_private?,
//!   context?: { auto_compact?, compact_at? } }`, the settings that belong to no account.
//! - `GET /api/ai/accounts` (scope `read`) answers `{ accounts: [{ id, name, kind, kind_label,
//!   model, configured, window, usage }], default, kinds, confirm, web, context }`, the chat panel's
//!   picker in every client (ai D23): no key, URL, header or sign-in.
//! - `GET /api/ai/models?account=` (scope `read`) lists the account's models live, the default
//!   account's without one, with its model, or the preselected one when it names none.
//!
//! The chat's system prompt and its `confirm` and `web` switches are the built-in Assistant's,
//! `agents/assistant.wcl` (agents G6): both GETs answer them from it, and `PUT .../settings`
//! writes the ones it is sent into it, checked as `PUT /api/agents/assistant` checks it.
//!
//! Credential resolution and the sign-in calls block, so they run in `spawn_blocking`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ai::{
    Account, AccountContext, AccountInput, Accounts, AiError, ChatGptAuth, Credentials, ErrorKind,
    Kind, PlanUsage, Settings, siwc,
};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
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
    /// The account signing in.
    account: String,
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

/// Run blocking work against the writer off the async runtime.
async fn blocking_w<T: Send + 'static>(
    a: &Arc<Ai>,
    f: impl FnOnce(&Writer) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let w = a.writer.clone();
    tokio::task::spawn_blocking(move || f(&w))
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

/// A refused account change: a 400, `invalid_url` when the URL was the trouble (ai D24).
fn account_error(e: AiError) -> ApiError {
    if e.message.starts_with("The API URL") {
        return ApiError::new(StatusCode::BAD_REQUEST, "invalid_url", e.message);
    }
    api_error(e)
}

/// A refused ChatGPT sign-in step: always a 400 the card shows as it is.
fn chatgpt_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "chatgpt", message)
}

pub(crate) fn now_ms() -> i64 {
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

/// The accounts, or an empty table when the vault has no secrets key, which holds none.
fn accounts_of(s: &state::State) -> Result<Accounts, ApiError> {
    if !s.has_secret_key() {
        return Ok(Accounts::default());
    }
    Accounts::load(s).map_err(api_error)
}

/// The account `r` names, by id or name, or a 404.
fn account(all: &Accounts, r: &str) -> Result<Account, ApiError> {
    all.find(r).cloned().ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("there is no AI account {r:?}"),
        )
    })
}

/// The built-in Assistant, which holds the chat panel's prompt and switches (agents G6): the
/// agent file, or every default when the vault has none yet.
pub(crate) fn assistant(w: &Writer) -> Result<schema::Agent, ApiError> {
    match w.agent(schema::ASSISTANT_AGENT) {
        Ok(a) => Ok(a),
        Err(writer::Error::Missing(_)) => Ok(schema::Agent::new(schema::ASSISTANT_AGENT)),
        Err(e) => Err(e.into()),
    }
}

/// An agent's confirmation switches with every key filled (ai D22).
pub(crate) fn confirm_json(c: &schema::AgentConfirm) -> Value {
    Value::Object(
        c.switches()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), json!(v)))
            .collect(),
    )
}

/// An agent's internet switches with every key filled (ai D28).
pub(crate) fn web_json(web: &schema::AgentWeb) -> Value {
    Value::Object(
        web.switches()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), json!(v)))
            .collect(),
    )
}

/// The account each agent names, for the `agents` of an account and its rename: `(agent,
/// account id)` for every agent naming an account that exists (accounts A6).
fn agents_by_account(w: &Writer, all: &Accounts) -> Vec<(String, String)> {
    let Ok(agents) = w.agents() else {
        return Vec::new();
    };
    agents
        .into_iter()
        .filter_map(|(name, a)| {
            let a = a.ok()?;
            let id = all.find(a.account_ref()?)?.id.clone();
            Some((name, id))
        })
        .collect()
}

/// The name of the account agent `a` names, resolved through the accounts: a legacy provider
/// reads as the name of the account made for it. The name as written when no account has it.
pub(crate) fn agent_account_name(w: &Writer, a: &schema::Agent) -> Option<String> {
    let r = a.account_ref()?;
    let all = accounts_of(w.state()).unwrap_or_default();
    Some(
        all.find(r)
            .map_or_else(|| r.to_owned(), |acc| acc.name.clone()),
    )
}

/// One account as `GET /api/integrations/ai` answers it. Nothing in it is a key or a token.
fn account_json(
    s: &state::State,
    a: &Account,
    agents: &[(String, String)],
) -> Result<Value, ApiError> {
    let configured = Credentials::configured(s, a).map_err(api_error)?;
    let key_set = a.kind != Kind::ChatGpt && Credentials::key_set(s, &a.id).map_err(api_error)?;
    let signed_in = match a.kind {
        Kind::ChatGpt => {
            let auth = ChatGptAuth::load(s, &a.id).map_err(api_error)?;
            (auth.signed_in() || auth.account.is_some())
                .then(|| json!({ "account": auth.account, "expired": !auth.signed_in() }))
        }
        _ => None,
    };
    let usage = match a.kind {
        Kind::ChatGpt => PlanUsage::load(s, &a.id).map_err(api_error)?,
        _ => None,
    };
    let named: Vec<&str> = agents
        .iter()
        .filter(|(_, id)| *id == a.id)
        .map(|(n, _)| n.as_str())
        .collect();
    Ok(json!({
        "id": a.id,
        "name": a.name,
        "kind": a.kind,
        "kind_label": a.kind.label(),
        "base_url": a.url(),
        "default_base_url": ai::default_base(a.kind),
        "model": a.model(),
        "headers": a.headers,
        "context": a.context,
        "default_window": ai::default_window(a.kind),
        "window": a.window(),
        "configured": configured,
        "key_set": key_set,
        "signed_in": signed_in,
        "agents": named,
        "usage": usage,
    }))
}

/// What `GET /api/integrations/ai` answers. Nothing in it is a key or a token. The prompt and the
/// switches are the built-in Assistant's (agents G6).
fn status_of(w: &Writer) -> Result<Value, ApiError> {
    let s = w.state();
    let agent = assistant(w)?;
    let custom = agent.system_prompt.clone().filter(|p| !p.trim().is_empty());
    let prompt = custom.clone().unwrap_or_else(|| ai::CHAT_PROMPT.to_owned());
    let keyed = s.has_secret_key();
    let settings = if keyed {
        Settings::load(s).map_err(api_error)?
    } else {
        Settings::default()
    };
    let all = accounts_of(s)?;
    let agents = agents_by_account(w, &all);
    let accounts = all
        .accounts
        .iter()
        .map(|a| account_json(s, a, &agents))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "secrets_key": keyed,
        "accounts": accounts,
        "default": all.default_account().map(|a| a.id.clone()),
        "kinds": kinds_json(),
        "context": context_json(&settings),
        "confirm": confirm_json(&agent.permissions.confirm),
        "web": web_json(&agent.permissions.web),
        "fetch_private": settings.fetch_private,
        "system_prompt": prompt,
        "default_system_prompt": ai::CHAT_PROMPT,
        "system_prompt_custom": custom.is_some(),
    }))
}

/// The account kinds in the order Settings > AI offers them (accounts A1): each kind's name,
/// label, default base URL (null for `openai_compatible`, whose URL is required) and default
/// context window.
fn kinds_json() -> Value {
    Value::Array(
        Kind::ALL
            .iter()
            .map(|&k| {
                json!({
                    "kind": k,
                    "label": k.label(),
                    "default_url": ai::default_base(k),
                    "default_window": ai::default_window(k),
                })
            })
            .collect(),
    )
}

/// Whether threads compact themselves, and at what share of the window.
fn context_json(settings: &Settings) -> Value {
    json!({
        "auto_compact": settings.context.auto_compact,
        "compact_at": settings.context.compact_at(),
    })
}

async fn status_answer(a: &Arc<Ai>) -> Result<Json<Value>, ApiError> {
    blocking_w(a, status_of).await.map(Json)
}

/// `GET /api/integrations/ai`.
pub async fn status(State(a): Shared) -> Result<Json<Value>, ApiError> {
    status_answer(&a).await
}

/// `GET /api/ai/accounts` (read): the accounts set up with their default models, the default
/// account and the confirmation switches, for the chat panel's picker in every client (ai D23).
/// Unlike `GET /api/integrations/ai` it holds no URL, header, sign-in or system prompt.
pub async fn accounts(State(a): Shared) -> Result<Json<Value>, ApiError> {
    let status = blocking_w(&a, status_of).await?;
    let accounts: Vec<Value> = status["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            json!({
                "id": a["id"], "name": a["name"], "kind": a["kind"], "kind_label": a["kind_label"],
                "model": a["model"], "configured": a["configured"], "window": a["window"],
                "usage": a["usage"],
            })
        })
        .collect();
    Ok(Json(json!({
        "accounts": accounts,
        "default": status["default"],
        "kinds": status["kinds"],
        "confirm": status["confirm"],
        "web": status["web"],
        "context": status["context"],
    })))
}

/// A header pair as the routes take it: `[name, value]`.
type HeaderPair = (String, String);

/// A new account (`POST`) or a change to one (`PUT`). `Some(None)` is a field sent as null.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AccountBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, with = "double_option")]
    base_url: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    model: Option<Option<String>>,
    #[serde(default)]
    headers: Option<Vec<HeaderPair>>,
    #[serde(default, with = "double_option")]
    context: Option<Option<AccountContext>>,
    /// `POST` only: the API key to store with the new account.
    #[serde(default)]
    key: Option<String>,
}

/// A field that tells absent (`None`) from null (`Some(None)`).
mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

impl AccountBody {
    fn input(self) -> Result<(AccountInput, Option<String>), ApiError> {
        let kind = match self.kind.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(k) => Some(k.parse::<Kind>().map_err(ApiError::bad_request)?),
        };
        let text = |v: Option<Option<String>>| {
            v.map(|v| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()))
        };
        Ok((
            AccountInput {
                name: self.name,
                kind,
                base_url: text(self.base_url),
                model: text(self.model),
                headers: self.headers,
                context: self.context,
            },
            self.key
                .map(|k| k.trim().to_owned())
                .filter(|k| !k.is_empty()),
        ))
    }
}

/// `POST /api/integrations/ai/accounts`.
pub async fn create_account(
    State(a): Shared,
    Json(b): Json<AccountBody>,
) -> Result<Json<Value>, ApiError> {
    let (input, key) = b.input()?;
    blocking_w(&a, move |w| {
        let s = w.state();
        let added = Accounts::change(s, api_error, |all| {
            let added = all.add(input).map_err(account_error)?.clone();
            if key.is_some() && added.kind == Kind::ChatGpt {
                return Err(ApiError::bad_request(
                    "a ChatGPT subscription takes no key: Continue with ChatGPT instead",
                ));
            }
            if let Some(k) = &key {
                s.set_secret(&ai::key_secret(&added.id), k)?;
            }
            Ok(added)
        })?;
        account_json(s, &added, &[])
    })
    .await
    .map(Json)
}

/// `PUT /api/integrations/ai/accounts/{id}`.
pub async fn update_account(
    State(a): Shared,
    crate::Caller(by): crate::Caller,
    Path(id): Path<String>,
    Json(b): Json<AccountBody>,
) -> Result<Json<Value>, ApiError> {
    if b.key.is_some() {
        return Err(ApiError::bad_request(
            "set the key with PUT /api/integrations/ai/accounts/{id}/key",
        ));
    }
    let (input, _) = b.input()?;
    blocking_w(&a, move |w| {
        let s = w.state();
        Accounts::change(s, api_error, |all| {
            let id = account(all, &id)?.id;
            let renamed = all.update(&id, input).map_err(account_error)?;
            if let Some(old) = renamed {
                let new = all.get(&id).map(|a| a.name.clone()).unwrap_or_default();
                let _acting = writer::acting_as(&by);
                // Every agent naming the old name, or the legacy provider this account was
                // made for, names the new one (accounts A6), before the rename is saved.
                let legacy = all.legacy_names(&id);
                w.set_agent_accounts(|agent| {
                    let r = agent.account_ref()?;
                    (r.eq_ignore_ascii_case(&old) && agent.account.is_some()
                        || agent.account.is_none() && legacy.iter().any(|l| l == r))
                    .then(|| new.clone())
                })?;
            }
            Ok(())
        })?;
        status_of(w)
    })
    .await
    .map(Json)
}

/// `DELETE /api/integrations/ai/accounts/{id}`: the account and its secrets. Agents naming it
/// fall back to the default account.
pub async fn delete_account(
    State(a): Shared,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let gone = blocking(&a, move |s| {
        Accounts::change(s, api_error, |all| {
            let id = account(all, &id)?.id;
            all.remove(&id).map_err(api_error)
        })
    })
    .await?;
    if gone.kind == Kind::ChatGpt {
        revoke(&a, &gone).await;
    }
    blocking_w(&a, move |w| {
        let s = w.state();
        for name in [
            ai::key_secret(&gone.id),
            ai::chatgpt_secret(&gone.id),
            ai::usage_secret(&gone.id),
        ] {
            delete_secret(s, &name)?;
        }
        status_of(w)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderBody {
    ids: Vec<String>,
}

/// `PUT /api/integrations/ai/accounts/order`.
pub async fn put_order(
    State(a): Shared,
    Json(b): Json<OrderBody>,
) -> Result<Json<Value>, ApiError> {
    blocking_w(&a, move |w| {
        Accounts::change(w.state(), api_error, |all| {
            all.reorder(&b.ids).map_err(api_error)
        })?;
        status_of(w)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultBody {
    #[serde(default)]
    id: Option<String>,
}

/// `PUT /api/integrations/ai/default`.
pub async fn put_default(
    State(a): Shared,
    Json(b): Json<DefaultBody>,
) -> Result<Json<Value>, ApiError> {
    blocking_w(&a, move |w| {
        Accounts::change(w.state(), api_error, |all| {
            all.set_default(b.id.as_deref()).map_err(api_error)
        })?;
        status_of(w)
    })
    .await
    .map(Json)
}

/// A partial `context`: each switch sent replaces the stored one.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ContextBody {
    #[serde(default)]
    auto_compact: Option<bool>,
    #[serde(default)]
    compact_at: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBody {
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    confirm: Option<ConfirmBody>,
    #[serde(default)]
    web: Option<WebBody>,
    #[serde(default)]
    fetch_private: Option<bool>,
    #[serde(default)]
    context: Option<ContextBody>,
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
    #[serde(default)]
    downloads: Option<bool>,
    #[serde(default)]
    taxonomies: Option<bool>,
    #[serde(default)]
    agents: Option<bool>,
    #[serde(default)]
    skills: Option<bool>,
    #[serde(default)]
    tools: Option<bool>,
}

/// A partial `web`: each internet tool switch sent replaces the stored one (ai D28).
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WebBody {
    #[serde(default)]
    search: Option<bool>,
    #[serde(default)]
    read: Option<bool>,
    #[serde(default)]
    download: Option<bool>,
}

/// `PUT /api/integrations/ai/settings`.
pub async fn put_settings(
    State(a): Shared,
    crate::Caller(by): crate::Caller,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    let prompt = b.system_prompt;
    let confirm = b.confirm;
    let web = b.web;
    let fetch_private = b.fetch_private;
    let context = b.context.unwrap_or_default();
    if let Some(n) = context.compact_at
        && !ai::Context::COMPACT_RANGE.contains(&n)
    {
        return Err(ApiError::bad_request(format!(
            "compact_at is from {} to {} percent",
            ai::Context::COMPACT_RANGE.start(),
            ai::Context::COMPACT_RANGE.end()
        )));
    }
    blocking_w(&a, move |w| {
        let _acting = writer::acting_as(&by);
        let s = w.state();
        // The chat's prompt and switches are the built-in Assistant's (agents G6): written
        // there first, so a refused prompt changes nothing.
        if prompt.is_some() || confirm.is_some() || web.is_some() {
            let mut agent = assistant(w)?;
            if let Some(c) = confirm {
                let to = &mut agent.permissions.confirm;
                for (slot, value) in [
                    (&mut to.notes, c.notes),
                    (&mut to.archive, c.archive),
                    (&mut to.delete, c.delete),
                    (&mut to.types, c.types),
                    (&mut to.downloads, c.downloads),
                    (&mut to.taxonomies, c.taxonomies),
                    (&mut to.skills, c.skills),
                    (&mut to.agents, c.agents),
                    (&mut to.tools, c.tools),
                ] {
                    if let Some(v) = value {
                        *slot = v;
                    }
                }
            }
            if let Some(b) = web {
                let to = &mut agent.permissions.web;
                for (slot, value) in [
                    (&mut to.search, b.search),
                    (&mut to.read, b.read),
                    (&mut to.download, b.download),
                ] {
                    if let Some(v) = value {
                        *slot = v;
                    }
                }
            }
            if let Some(p) = prompt {
                let mut check = Settings::default();
                check.set_system_prompt(&p).map_err(api_error)?;
                agent.system_prompt = check.system_prompt;
            }
            let check = crate::agent_files::tool_check(w);
            w.put_agent(schema::ASSISTANT_AGENT, &agent.to_wcl(), check)?;
        }
        if fetch_private.is_some() || context.auto_compact.is_some() || context.compact_at.is_some()
        {
            let mut settings = Settings::load(s).map_err(api_error)?;
            if let Some(on) = fetch_private {
                settings.fetch_private = on;
            }
            if let Some(on) = context.auto_compact {
                settings.context.auto_compact = on;
            }
            if let Some(n) = context.compact_at {
                settings.context.compact_at = (n != ai::Context::COMPACT_AT).then_some(n);
            }
            settings.save(s).map_err(api_error)?;
        }
        status_of(w)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBody {
    key: String,
}

/// `PUT /api/integrations/ai/accounts/{id}/key`.
pub async fn put_key(
    State(a): Shared,
    Path(id): Path<String>,
    Json(b): Json<KeyBody>,
) -> Result<Json<Value>, ApiError> {
    let key = b.key.trim().to_owned();
    blocking_w(&a, move |w| {
        let s = w.state();
        let acc = account(&Accounts::load(s).map_err(api_error)?, &id)?;
        if acc.kind == Kind::ChatGpt {
            return Err(ApiError::bad_request(
                "a ChatGPT subscription takes no key: Continue with ChatGPT instead",
            ));
        }
        let secret = ai::key_secret(&acc.id);
        if key.is_empty() {
            delete_secret(s, &secret)?;
        } else {
            s.set_secret(&secret, &key)?;
        }
        status_of(w)
    })
    .await
    .map(Json)
}

/// The `chatgpt` account `id` names, or a 400.
fn chatgpt_account(s: &state::State, id: &str) -> Result<Account, ApiError> {
    let acc = account(&Accounts::load(s).map_err(api_error)?, id)?;
    if acc.kind != Kind::ChatGpt {
        return Err(ApiError::bad_request(format!(
            "{} is not a ChatGPT subscription account",
            acc.name
        )));
    }
    Ok(acc)
}

/// `POST /api/integrations/ai/accounts/{id}/chatgpt/start`.
pub async fn chatgpt_start(
    State(a): Shared,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (acc, auth, auth_base) = blocking(&a, move |s| {
        let acc = chatgpt_account(s, &id)?;
        let mut auth = ChatGptAuth::load(s, &acc.id).map_err(api_error)?;
        if auth.host_id.as_deref().is_none_or(str::is_empty) {
            auth.host_id = Some(siwc::new_host_id());
            auth.save(s, &acc.id).map_err(api_error)?;
        }
        let settings = Settings::load(s).map_err(api_error)?;
        Ok((acc, auth, settings.endpoints.chatgpt_auth_base()))
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
            account: acc.id,
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

/// `POST /api/integrations/ai/accounts/{id}/chatgpt/finish`.
pub async fn chatgpt_finish(
    State(a): Shared,
    Path(id): Path<String>,
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

    blocking_w(&a, move |w| {
        let s = w.state();
        let acc = chatgpt_account(s, &id)?;
        if acc.id != pending.account {
            return Err(chatgpt_error(
                "this sign-in was started for another account: Continue with ChatGPT again",
            ));
        }
        let mut auth = ChatGptAuth::load(s, &acc.id).map_err(api_error)?;
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
        auth.save(s, &acc.id).map_err(api_error)?;
        status_of(w)
    })
    .await
    .map(Json)
}

/// Revoke `acc`'s ChatGPT refresh token, best effort.
async fn revoke(a: &Arc<Ai>, acc: &Account) {
    let id = acc.id.clone();
    let done = blocking(a, move |s| {
        let auth = ChatGptAuth::load(s, &id).map_err(api_error)?;
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
        Ok(())
    })
    .await;
    if let Err(e) = done {
        eprintln!("ai: revoking the ChatGPT sign-in: {}", e.status);
    }
}

/// `POST /api/integrations/ai/accounts/{id}/chatgpt/disconnect`.
pub async fn chatgpt_disconnect(
    State(a): Shared,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let acc = blocking(&a, move |s| chatgpt_account(s, &id)).await?;
    revoke(&a, &acc).await;
    blocking_w(&a, move |w| {
        let s = w.state();
        delete_secret(s, &ai::chatgpt_secret(&acc.id))?;
        delete_secret(s, &ai::usage_secret(&acc.id))?;
        status_of(w)
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

/// The account `r` names, the default without one, and its credential.
async fn credential(
    a: &Arc<Ai>,
    r: Option<String>,
) -> Result<(Account, Result<ai::Credential, AiError>), ApiError> {
    blocking(a, move |s| {
        let all = Accounts::load(s).map_err(api_error)?;
        let acc = match r.as_deref() {
            Some(r) => account(&all, r)?,
            None => all.pick(None, None).map_err(api_error)?.clone(),
        };
        let cred = Credentials::resolve(s, &acc);
        Ok((acc, cred))
    })
    .await
}

/// Keep the plan usage a reply reported as `account`'s latest (accounts A13).
pub(crate) fn keep_usage(s: &state::State, account: &str, usage: Option<&PlanUsage>) {
    if let Some(u) = usage
        && !account.is_empty()
        && let Err(e) = u.save(s, account)
    {
        eprintln!("ai: keeping the plan usage: {e}");
    }
}

/// `POST /api/integrations/ai/accounts/{id}/test`.
pub async fn test(
    State(a): Shared,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let b: TestBody = if body.iter().all(u8::is_ascii_whitespace) {
        TestBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?
    };
    let (acc, cred) = credential(&a, Some(id)).await?;
    let began = Instant::now();
    let answer = |model: Option<&str>, result: Result<String, AiError>| {
        let ms = began.elapsed().as_millis() as u64;
        let (ok, text, error) = match result {
            Ok(text) => (true, json!(text), Value::Null),
            Err(e) => (
                false,
                Value::Null,
                json!({ "kind": e.kind.as_str(), "message": e.message }),
            ),
        };
        json!({
            "ok": ok, "account": acc.id, "name": acc.name, "kind": acc.kind, "model": model,
            "ms": ms, "text": text, "error": error,
        })
    };
    let cred = match cred {
        Ok(c) => c,
        Err(e) => return Ok(Json(answer(None, Err(e)))),
    };
    let asked = b
        .model
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty());
    let model = match asked.or_else(|| acc.model().map(str::to_owned)) {
        Some(m) => m,
        None => {
            let models = match ai::list_models(&a.http, &cred).await {
                Ok(m) => m,
                Err(e) => return Ok(Json(answer(None, Err(e)))),
            };
            let Some(m) = ai::preselect(acc.kind, &models).map(|m| m.id.clone()) else {
                let e = AiError::not_configured(format!(
                    "no model is chosen for {}: pick one from the list",
                    acc.name
                ));
                return Ok(Json(answer(None, Err(e))));
            };
            let keep = m.clone();
            let id = acc.id.clone();
            blocking(&a, move |s| {
                Accounts::change(s, api_error, |all| {
                    if all.get(&id).is_some_and(|a| a.model().is_none()) {
                        all.update(
                            &id,
                            AccountInput {
                                model: Some(Some(keep)),
                                ..AccountInput::default()
                            },
                        )
                        .map_err(api_error)?;
                    }
                    Ok(())
                })
            })
            .await?;
            m
        }
    };
    let req = ai::Request::test(model.clone());
    let result = test_call(&a, &cred, &req).await;
    if let Ok(reply) = &result {
        let usage = reply.plan_usage.clone();
        let id = acc.id.clone();
        blocking(&a, move |s| {
            keep_usage(s, &id, usage.as_ref());
            Ok(())
        })
        .await?;
    }
    Ok(Json(answer(Some(&model), result.map(|r| r.text))))
}

/// One Test request, under [`TEST_TIMEOUT`], leaving its trail line.
async fn test_call(
    a: &Arc<Ai>,
    cred: &ai::Credential,
    req: &ai::Request,
) -> Result<ai::Reply, AiError> {
    let began = Instant::now();
    let result = match tokio::time::timeout(TEST_TIMEOUT, ai::complete(&a.http, cred, req)).await {
        Ok(r) => r,
        Err(_) => Err(AiError::new(
            ErrorKind::Timeout,
            format!(
                "{} did not answer within {} seconds",
                cred.name,
                TEST_TIMEOUT.as_secs()
            ),
        )),
    };
    eprintln!(
        "{}",
        ai::trail_for(cred, &req.model, &result, began.elapsed())
    );
    result
}

#[derive(Deserialize)]
pub struct ModelsQuery {
    #[serde(default)]
    account: Option<String>,
}

/// `GET /api/ai/models?account=`.
pub async fn models(
    State(a): Shared,
    Query(q): Query<ModelsQuery>,
) -> Result<Json<Value>, ApiError> {
    let r = q
        .account
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty());
    let (acc, cred) = credential(&a, r).await?;
    let cred = cred.map_err(api_error)?;
    let list = ai::list_models(&a.http, &cred).await.map_err(api_error)?;
    let model = acc
        .model()
        .map(str::to_owned)
        .or_else(|| ai::preselect(acc.kind, &list).map(|m| m.id.clone()));
    Ok(Json(
        json!({ "account": acc.id, "model": model, "models": list }),
    ))
}
