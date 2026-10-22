//! The `ai_settings` secret (ai D7): the default provider, the default model per provider, and
//! the base URLs the clients call, which tests point at the testkit fakes.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{AiError, ErrorKind};
use crate::types::Provider;

/// The secret holding [`Settings`].
pub const SETTINGS: &str = "ai_settings";

/// Anthropic's API, which `/v1/messages` and `/v1/models` hang off.
pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com";

/// OpenAI's API, for an API key and a ChatGPT plan token alike (ai D1).
pub const OPENAI_BASE: &str = "https://api.openai.com";

/// OpenAI's sign-in server for Sign in with ChatGPT (ai D8).
pub const CHATGPT_AUTH_BASE: &str = "https://auth.openai.com";

/// Anthropic's model preselected when the live list has it (ai D6).
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5";

/// The default model of each provider. `None` means the user has not picked one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Models {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt: Option<String>,
}

/// Base URLs, without a trailing slash, in place of the real hosts. Absent means the real one.
/// The `anthropic` base takes `/v1/messages` and `/v1/models`; the `openai` base takes
/// `/v1/responses` and `/v1/models`, for the ChatGPT plan too; the `chatgpt_auth` base takes
/// `/api/accounts/oauth/token` and `/.well-known/openid-configuration`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_auth: Option<String>,
}

impl Endpoints {
    /// The base URL `provider`'s inference and model calls go to.
    pub fn api_base(&self, provider: Provider) -> String {
        let (set, real) = match provider {
            Provider::Anthropic => (&self.anthropic, ANTHROPIC_BASE),
            Provider::OpenAi | Provider::ChatGpt => (&self.openai, OPENAI_BASE),
        };
        base(set.as_deref(), real)
    }

    /// The base URL of the ChatGPT sign-in server.
    pub fn chatgpt_auth_base(&self) -> String {
        base(self.chatgpt_auth.as_deref(), CHATGPT_AUTH_BASE)
    }
}

fn base(set: Option<&str>, real: &str) -> String {
    match set.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.trim_end_matches('/').to_owned(),
        None => real.to_owned(),
    }
}

/// The `ai_settings` secret. Keys this build does not know are kept through a load and save.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<Provider>,
    #[serde(default)]
    pub models: Models,
    #[serde(default, skip_serializing_if = "is_default")]
    pub endpoints: Endpoints,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

fn is_default(e: &Endpoints) -> bool {
    *e == Endpoints::default()
}

impl Settings {
    /// The stored settings, or the defaults when there are none. A secret that is not a
    /// settings object counts as none.
    pub fn load(state: &state::State) -> Result<Settings, AiError> {
        Ok(match state.get_secret(SETTINGS).map_err(state_error)? {
            Some(text) => serde_json::from_str(&text).unwrap_or_default(),
            None => Settings::default(),
        })
    }

    pub fn save(&self, state: &state::State) -> Result<(), AiError> {
        let text = serde_json::to_string(self).map_err(|e| AiError::invalid(e.to_string()))?;
        state.set_secret(SETTINGS, &text).map_err(state_error)
    }

    /// The default model chosen for `provider`.
    pub fn model(&self, provider: Provider) -> Option<&str> {
        match provider {
            Provider::Anthropic => self.models.anthropic.as_deref(),
            Provider::OpenAi => self.models.openai.as_deref(),
            Provider::ChatGpt => self.models.chatgpt.as_deref(),
        }
        .filter(|m| !m.trim().is_empty())
    }

    pub fn set_model(&mut self, provider: Provider, model: Option<String>) {
        let slot = match provider {
            Provider::Anthropic => &mut self.models.anthropic,
            Provider::OpenAi => &mut self.models.openai,
            Provider::ChatGpt => &mut self.models.chatgpt,
        };
        *slot = model.filter(|m| !m.trim().is_empty());
    }

    /// The provider and model a call uses: the ones asked for, else the defaults. Errors with
    /// `not_configured` when no provider or model is chosen.
    pub fn pick(
        &self,
        provider: Option<Provider>,
        model: Option<&str>,
    ) -> Result<(Provider, String), AiError> {
        let provider = provider.or(self.default_provider).ok_or_else(|| {
            AiError::not_configured("no AI provider is chosen: pick one in Settings > AI")
        })?;
        let model = model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .or_else(|| self.model(provider))
            .ok_or_else(|| {
                AiError::not_configured(format!(
                    "no {} model is chosen: pick one in Settings > AI",
                    provider.label()
                ))
                .on(provider, "")
            })?;
        Ok((provider, model.to_owned()))
    }
}

/// A state error, which is never about the provider.
pub(crate) fn state_error(e: state::Error) -> AiError {
    match e {
        state::Error::KeyMissing { .. } => AiError::new(
            ErrorKind::NotConfigured,
            format!("the AI credentials cannot be read: {e}"),
        ),
        e => AiError::new(
            ErrorKind::Unavailable,
            format!("the AI settings cannot be read or written: {e}"),
        ),
    }
}
