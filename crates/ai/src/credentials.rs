//! Credentials from the encrypted secrets (ai D7), per account (accounts A4), and the ChatGPT
//! plan's token refresh (ai D9).
//!
//! - `ai_account_key_<id>` holds an account's raw API key.
//! - `ai_account_chatgpt_<id>` holds a `chatgpt` account's [`ChatGptAuth`]: the issued client id,
//!   the host id, the token pair, their expiry, the ID token and the plan account's address.
//!
//! [`Credentials::resolve`] hands out what a call needs. For the ChatGPT plan it first refreshes
//! the access token when under [`REFRESH_MARGIN_MS`] of it is left (and not before
//! `earliest_refresh_ms`). The refresh token rotates, so the refresh runs under one process-wide
//! lock and the rotated pair is written back before the lock is released: a second caller waits,
//! reads the new pair and uses it. A refresh answered `invalid_grant` deletes that account's
//! tokens, keeps the client id and host id, adds one bell notification naming the account and
//! returns an `auth` error. Later calls find no tokens and return `not_configured` without
//! notifying again.

use std::fmt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::accounts::{Account, chatgpt_secret, key_secret};
use crate::error::{AiError, ErrorKind};
use crate::settings::{Settings, state_error};
use crate::siwc;
use crate::types::{Kind, ProviderState, Wire};

/// The Settings page the expiry notification opens.
pub const SETTINGS_URL: &str = "/settings/ai";

/// The title of the notification a dead ChatGPT sign-in adds.
pub const EXPIRED_TITLE: &str = "ChatGPT sign-in expired";

/// How close to expiry an access token is refreshed: 5 minutes.
pub const REFRESH_MARGIN_MS: i64 = 5 * 60 * 1000;

/// One refresh at a time in the whole process (ai D9).
static REFRESH: Mutex<()> = Mutex::new(());

/// A key or token. Its `Debug` and `Display` never show it; [`Secret::expose`] does.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The value, for an `Authorization` header or a token request and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// What a call needs: which account it is, where to send it and the key or access token.
#[derive(Clone, Debug)]
pub struct Credential {
    pub kind: Kind,
    /// The account's id; empty for a credential made by hand ([`Credential::new`]).
    pub account: String,
    /// The account's name, which error messages and the trail line use.
    pub name: String,
    /// The provider names of before accounts that map to this account (accounts A8): a saved
    /// turn naming one of them replays as this account's.
    pub legacy: Vec<String>,
    /// The API base URL: the account's, else the kind's.
    pub base_url: String,
    /// The key or token. Empty for an account with its own base URL and no key, which sends no
    /// auth header (ai D24).
    pub secret: Secret,
    /// The ChatGPT plan account's address, for a `chatgpt` account.
    pub signed_in_as: Option<String>,
    /// The host of the account's own base URL, for the trail line and the compatibility errors
    /// (ai D24). `None` for the kind's own host.
    pub custom_host: Option<String>,
    /// Extra headers sent on every request (accounts A3).
    pub headers: Vec<(String, String)>,
}

impl Credential {
    /// A credential of `kind` with no account behind it, named by the kind's short name.
    pub fn new(kind: Kind, base_url: impl Into<String>, secret: Secret) -> Credential {
        Credential {
            kind,
            account: String::new(),
            name: kind.short().to_owned(),
            legacy: Vec::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            secret,
            signed_in_as: None,
            custom_host: None,
            headers: Vec::new(),
        }
    }

    /// The wire this credential's calls speak.
    pub fn wire(&self) -> Wire {
        self.kind.wire()
    }

    /// Whether calls go to `/chat/completions` (ai D25).
    pub fn chat_completions(&self) -> bool {
        self.wire() == Wire::ChatCompletions
    }

    /// Whether `state` may be replayed on a request to `model` with this credential: the same
    /// account, by id or by a legacy provider name, the same wire and the same model (accounts
    /// A8).
    pub fn replays(&self, state: &ProviderState, model: &str) -> bool {
        state.wire == self.wire()
            && state.model == model
            && (state.account == self.account || self.legacy.contains(&state.account))
    }
}

/// A `chatgpt` account's sign-in, `ai_account_chatgpt_<id>` (ai D7, accounts A4). Keys this build does not know are kept through a load and
/// save.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatGptAuth {
    /// The issued `oaiapp_...` client id, kept across sign-ins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// `urn:uuid:<v4>`, made once ([`siwc::new_host_id`]) and kept across sign-ins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<Secret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<Secret>,
    /// When the access token expires, in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<i64>,
    /// The earliest a refresh is allowed, in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_refresh_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<Secret>,
    /// The account's address, from the ID token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl fmt::Debug for ChatGptAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatGptAuth")
            .field("client_id", &self.client_id)
            .field("host_id", &self.host_id)
            .field("access_token", &self.access_token)
            .field("refresh_token", &self.refresh_token)
            .field("expires_ms", &self.expires_ms)
            .field("earliest_refresh_ms", &self.earliest_refresh_ms)
            .field("id_token", &self.id_token)
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl ChatGptAuth {
    /// Account `id`'s stored record, or an empty one when there is none.
    pub fn load(state: &state::State, id: &str) -> Result<ChatGptAuth, AiError> {
        Ok(
            match state.get_secret(&chatgpt_secret(id)).map_err(state_error)? {
                Some(text) => serde_json::from_str(&text).unwrap_or_default(),
                None => ChatGptAuth::default(),
            },
        )
    }

    pub fn save(&self, state: &state::State, id: &str) -> Result<(), AiError> {
        let text = serde_json::to_string(self).map_err(|e| AiError::invalid(e.to_string()))?;
        state
            .set_secret(&chatgpt_secret(id), &text)
            .map_err(state_error)
    }

    /// Whether it holds a token pair to call with.
    pub fn signed_in(&self) -> bool {
        self.access_token.as_ref().is_some_and(|t| !t.is_empty())
            && self.refresh_token.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// Take the tokens of a code exchange or refresh received at `now_ms`. A refresh that
    /// returns no new refresh token keeps the old one.
    pub fn store_tokens(&mut self, tokens: siwc::TokenSet, now_ms: i64) {
        self.access_token = Some(tokens.access_token);
        if let Some(r) = tokens.refresh_token {
            self.refresh_token = Some(r);
        }
        if let Some(id) = tokens.id_token {
            self.id_token = Some(id);
        }
        self.expires_ms = tokens
            .expires_in
            .map(|s| now_ms.saturating_add((s as i64).saturating_mul(1000)));
        self.earliest_refresh_ms = tokens.earliest_refresh_ms;
    }

    /// Forget the tokens, keeping the client id, the host id and the account, which the next
    /// sign-in reuses.
    pub fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.expires_ms = None;
        self.earliest_refresh_ms = None;
        self.id_token = None;
    }

    /// Whether the access token has under [`REFRESH_MARGIN_MS`] left at `now_ms` and a refresh
    /// is allowed. A token with no known expiry is refreshed only once it is refused.
    pub fn needs_refresh(&self, now_ms: i64) -> bool {
        let Some(expires) = self.expires_ms else {
            return false;
        };
        let soon = now_ms.saturating_add(REFRESH_MARGIN_MS) >= expires;
        let allowed = now_ms >= expires || self.earliest_refresh_ms.is_none_or(|e| now_ms >= e);
        soon && allowed
    }
}

/// Where calls get their credentials.
pub struct Credentials;

impl Credentials {
    /// The credential for `account`. A ChatGPT plan token close to expiry is refreshed first
    /// (see the module docs). Errors with `not_configured` when the account has no key or
    /// sign-in, and `auth` when the ChatGPT sign-in has just expired. An `anthropic` or `openai`
    /// account with its own base URL and no key, and an `openai_compatible` account with no key,
    /// resolve to an empty secret, which sends no auth header (ai D24).
    ///
    /// It blocks on the refresh request: from async code call it inside
    /// `tokio::task::spawn_blocking`.
    pub fn resolve(state: &state::State, account: &Account) -> Result<Credential, AiError> {
        let settings = Settings::load(state)?;
        let all = crate::Accounts::load(state)?;
        let base = account.api_base(&settings.endpoints);
        let mut cred = Credential::new(account.kind, base, Secret::new(""));
        cred.account = account.id.clone();
        cred.name = account.name.clone();
        cred.legacy = all.legacy_names(&account.id);
        match account.kind {
            Kind::Anthropic | Kind::OpenAi | Kind::OpenAiCompatible => {
                let key = stored_key(state, &account.id)?;
                if key.is_none() && account.url().is_none() {
                    return Err(AiError::not_configured(format!(
                        "the AI account {} has no API key: add one in Settings > AI",
                        account.name
                    ))
                    .on(&account.name, ""));
                }
                cred.secret = Secret::new(key.unwrap_or_default());
                cred.custom_host = account.custom_host();
                if account.kind == Kind::OpenAiCompatible {
                    cred.headers = account.headers.clone();
                }
            }
            Kind::ChatGpt => {
                let auth =
                    resolve_chatgpt(state, account, &settings.endpoints.chatgpt_auth_base())?;
                cred.secret = auth.access_token.clone().unwrap_or_else(|| Secret::new(""));
                cred.signed_in_as = auth.account.clone();
            }
        }
        Ok(cred)
    }

    /// Whether `account` has a key, its own base URL (ai D24) or a ChatGPT sign-in, without
    /// refreshing anything.
    pub fn configured(state: &state::State, account: &Account) -> Result<bool, AiError> {
        Ok(match account.kind {
            Kind::ChatGpt => ChatGptAuth::load(state, &account.id)?.signed_in(),
            _ => account.url().is_some() || stored_key(state, &account.id)?.is_some(),
        })
    }

    /// Whether account `id` has an API key stored.
    pub fn key_set(state: &state::State, id: &str) -> Result<bool, AiError> {
        Ok(stored_key(state, id)?.is_some())
    }
}

/// Account `id`'s API key, trimmed, when one is stored.
fn stored_key(state: &state::State, id: &str) -> Result<Option<String>, AiError> {
    Ok(state
        .get_secret(&key_secret(id))
        .map_err(state_error)?
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty()))
}

fn not_signed_in(account: &Account) -> AiError {
    AiError::not_configured(format!(
        "the ChatGPT account {} is not signed in: Continue with ChatGPT in Settings > AI",
        account.name
    ))
    .on(&account.name, "")
}

fn resolve_chatgpt(
    state: &state::State,
    account: &Account,
    auth_base: &str,
) -> Result<ChatGptAuth, AiError> {
    let id = account.id.as_str();
    let auth = ChatGptAuth::load(state, id)?;
    if !auth.signed_in() {
        return Err(not_signed_in(account));
    }
    if !auth.needs_refresh(now_ms()) {
        return Ok(auth);
    }
    let _one = REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    // Another caller may have refreshed while this one waited.
    let mut auth = ChatGptAuth::load(state, id)?;
    if !auth.signed_in() {
        return Err(not_signed_in(account));
    }
    let now = now_ms();
    if !auth.needs_refresh(now) {
        return Ok(auth);
    }
    let (Some(client_id), Some(refresh)) = (auth.client_id.clone(), auth.refresh_token.clone())
    else {
        return Err(not_signed_in(account));
    };
    match siwc::refresh(
        &crate::http::blocking_client(),
        auth_base,
        &client_id,
        &refresh,
    ) {
        Ok(tokens) => {
            auth.store_tokens(tokens, now);
            auth.save(state, id)?;
            Ok(auth)
        }
        Err(siwc::RefreshError::InvalidGrant) => {
            auth.clear_tokens();
            auth.save(state, id)?;
            let note = state::NewNotification {
                title: EXPIRED_TITLE.to_owned(),
                body: Some(format!(
                    "TheVault can no longer use the ChatGPT plan of the account {}. Continue \
                     with ChatGPT again in Settings > AI.",
                    account.name
                )),
                note_id: None,
                url: Some(SETTINGS_URL.to_owned()),
                source: "system".to_owned(),
            };
            if let Err(e) = state.add_notification(&note) {
                eprintln!("ai: recording the ChatGPT sign-in notification: {e}");
            }
            Err(AiError::new(
                ErrorKind::Auth,
                format!(
                    "the ChatGPT sign-in of {} expired: Continue with ChatGPT again in Settings > AI",
                    account.name
                ),
            )
            .on(&account.name, ""))
        }
        // The old token still works for a while: use it, and refresh on a later call.
        Err(siwc::RefreshError::Other(_)) if auth.expires_ms.is_some_and(|e| now < e) => Ok(auth),
        Err(siwc::RefreshError::Other(e)) => Err(e),
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
