//! Credentials from the encrypted secrets (ai D7), and the ChatGPT plan's token refresh (ai D9).
//!
//! - `ai_anthropic_api_key` and `ai_openai_api_key` hold the raw keys.
//! - `ai_chatgpt` holds [`ChatGptAuth`]: the issued client id, the host id, the token pair, their
//!   expiry, the ID token and the account's address.
//!
//! [`Credentials::resolve`] hands out what a call needs. For the ChatGPT plan it first refreshes
//! the access token when under [`REFRESH_MARGIN_MS`] of it is left (and not before
//! `earliest_refresh_ms`). The refresh token rotates, so the refresh runs under one process-wide
//! lock and the rotated pair is written back before the lock is released: a second caller waits,
//! reads the new pair and uses it. A refresh answered `invalid_grant` deletes the tokens, keeps
//! the client id and host id, adds one bell notification and returns an `auth` error. Later
//! calls find no tokens and return `not_configured` without notifying again.

use std::fmt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{AiError, ErrorKind};
use crate::settings::{OpenAiApi, Settings, state_error};
use crate::siwc;
use crate::types::Provider;

/// The secret holding the Anthropic API key.
pub const ANTHROPIC_API_KEY: &str = "ai_anthropic_api_key";

/// The secret holding the OpenAI API key.
pub const OPENAI_API_KEY: &str = "ai_openai_api_key";

/// The secret holding [`ChatGptAuth`].
pub const CHATGPT: &str = "ai_chatgpt";

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

/// What a call to `provider` needs: where to send it and the key or access token.
#[derive(Clone, Debug)]
pub struct Credential {
    pub provider: Provider,
    /// The API base URL, from `ai_settings.endpoints` or the real host.
    pub base_url: String,
    /// The key or token. Empty for a custom API URL with no key, which sends no auth header
    /// (ai D24).
    pub secret: Secret,
    /// The ChatGPT account's address, for the ChatGPT plan.
    pub account: Option<String>,
    /// The host of a custom API URL, for the trail line and the compatibility errors (ai D24).
    /// `None` for the official API.
    pub custom_host: Option<String>,
    /// The API an OpenAI call speaks (ai D25). Only an OpenAI credential with a custom API URL
    /// uses Chat Completions: the official API and the ChatGPT plan always use Responses.
    pub openai_api: OpenAiApi,
}

impl Credential {
    pub fn new(provider: Provider, base_url: impl Into<String>, secret: Secret) -> Credential {
        Credential {
            provider,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            secret,
            account: None,
            custom_host: None,
            openai_api: OpenAiApi::Responses,
        }
    }

    /// The API this credential's calls speak: the stored type for OpenAI with a custom API URL,
    /// else Responses (ai D25). Anthropic's answer is meaningless.
    pub fn api(&self) -> OpenAiApi {
        match (self.provider, &self.custom_host) {
            (Provider::OpenAi, Some(_)) => self.openai_api,
            _ => OpenAiApi::Responses,
        }
    }

    /// Whether calls go to `/chat/completions` rather than `/responses` (ai D25).
    pub fn chat_completions(&self) -> bool {
        self.provider == Provider::OpenAi && self.api() == OpenAiApi::ChatCompletions
    }
}

/// The `ai_chatgpt` secret (ai D7). Keys this build does not know are kept through a load and
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
    /// The stored record, or an empty one when there is none.
    pub fn load(state: &state::State) -> Result<ChatGptAuth, AiError> {
        Ok(match state.get_secret(CHATGPT).map_err(state_error)? {
            Some(text) => serde_json::from_str(&text).unwrap_or_default(),
            None => ChatGptAuth::default(),
        })
    }

    pub fn save(&self, state: &state::State) -> Result<(), AiError> {
        let text = serde_json::to_string(self).map_err(|e| AiError::invalid(e.to_string()))?;
        state.set_secret(CHATGPT, &text).map_err(state_error)
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
    /// The credential for `provider`, with its base URL from `ai_settings.endpoints`. A ChatGPT
    /// plan token close to expiry is refreshed first (see the module docs). Errors with
    /// `not_configured` when the provider has no key or sign-in, and `auth` when the ChatGPT
    /// sign-in has just expired. Anthropic or OpenAI with a custom API URL and no key resolves
    /// to an empty secret, which sends no auth header (ai D24).
    ///
    /// It blocks on the refresh request: from async code call it inside
    /// `tokio::task::spawn_blocking`.
    pub fn resolve(state: &state::State, provider: Provider) -> Result<Credential, AiError> {
        let settings = Settings::load(state)?;
        let base = settings.endpoints.api_base(provider);
        match provider {
            Provider::Anthropic | Provider::OpenAi => {
                let custom = settings.endpoints.url(provider).is_some();
                let key = stored_key(state, provider)?;
                if key.is_none() && !custom {
                    return Err(AiError::not_configured(format!(
                        "no {} API key is set: add one in Settings > AI",
                        provider.label()
                    ))
                    .on(provider, ""));
                }
                let mut cred =
                    Credential::new(provider, base, Secret::new(key.unwrap_or_default()));
                cred.custom_host = settings.endpoints.custom_host(provider);
                if provider == Provider::OpenAi {
                    cred.openai_api = settings.endpoints.openai_api();
                }
                Ok(cred)
            }
            Provider::ChatGpt => {
                let auth = resolve_chatgpt(state, &settings.endpoints.chatgpt_auth_base())?;
                let mut cred = Credential::new(
                    provider,
                    base,
                    auth.access_token.clone().unwrap_or_else(|| Secret::new("")),
                );
                cred.account = auth.account.clone();
                Ok(cred)
            }
        }
    }

    /// Whether `provider` has a key, a custom API URL (ai D24) or a ChatGPT sign-in, without
    /// refreshing anything.
    pub fn configured(state: &state::State, provider: Provider) -> Result<bool, AiError> {
        Ok(match provider {
            Provider::Anthropic | Provider::OpenAi => {
                stored_key(state, provider)?.is_some()
                    || Settings::load(state)?.endpoints.url(provider).is_some()
            }
            Provider::ChatGpt => ChatGptAuth::load(state)?.signed_in(),
        })
    }
}

/// `provider`'s API key, trimmed, when one is stored.
fn stored_key(state: &state::State, provider: Provider) -> Result<Option<String>, AiError> {
    Ok(state
        .get_secret(key_secret(provider))
        .map_err(state_error)?
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty()))
}

/// The secret holding `provider`'s API key. The ChatGPT plan has none: it is [`CHATGPT`].
pub fn key_secret(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => ANTHROPIC_API_KEY,
        Provider::OpenAi | Provider::ChatGpt => OPENAI_API_KEY,
    }
}

fn not_signed_in() -> AiError {
    AiError::not_configured("ChatGPT is not signed in: Continue with ChatGPT in Settings > AI")
        .on(Provider::ChatGpt, "")
}

fn resolve_chatgpt(state: &state::State, auth_base: &str) -> Result<ChatGptAuth, AiError> {
    let auth = ChatGptAuth::load(state)?;
    if !auth.signed_in() {
        return Err(not_signed_in());
    }
    if !auth.needs_refresh(now_ms()) {
        return Ok(auth);
    }
    let _one = REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    // Another caller may have refreshed while this one waited.
    let mut auth = ChatGptAuth::load(state)?;
    if !auth.signed_in() {
        return Err(not_signed_in());
    }
    let now = now_ms();
    if !auth.needs_refresh(now) {
        return Ok(auth);
    }
    let (Some(client_id), Some(refresh)) = (auth.client_id.clone(), auth.refresh_token.clone())
    else {
        return Err(not_signed_in());
    };
    match siwc::refresh(
        &crate::http::blocking_client(),
        auth_base,
        &client_id,
        &refresh,
    ) {
        Ok(tokens) => {
            auth.store_tokens(tokens, now);
            auth.save(state)?;
            Ok(auth)
        }
        Err(siwc::RefreshError::InvalidGrant) => {
            auth.clear_tokens();
            auth.save(state)?;
            let note = state::NewNotification {
                title: EXPIRED_TITLE.to_owned(),
                body: Some(
                    "TheVault can no longer use your ChatGPT plan. Continue with ChatGPT again in \
                     Settings > AI."
                        .to_owned(),
                ),
                note_id: None,
                url: Some(SETTINGS_URL.to_owned()),
                source: "system".to_owned(),
            };
            if let Err(e) = state.add_notification(&note) {
                eprintln!("ai: recording the ChatGPT sign-in notification: {e}");
            }
            Err(AiError::new(
                ErrorKind::Auth,
                "the ChatGPT sign-in expired: Continue with ChatGPT again in Settings > AI",
            )
            .on(Provider::ChatGpt, ""))
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
