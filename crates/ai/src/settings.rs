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

/// The longest chat system prompt the settings take, in characters (ai D17).
pub const SYSTEM_PROMPT_MAX: usize = 32_000;

/// The `ai_settings` secret. Keys this build does not know are kept through a load and save.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<Provider>,
    #[serde(default)]
    pub models: Models,
    #[serde(default, skip_serializing_if = "is_default")]
    pub endpoints: Endpoints,
    /// The chat panel's system prompt (ai D17). Absent or empty means [`crate::CHAT_PROMPT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Which kinds of chat change wait for the user (ai D22).
    #[serde(default, skip_serializing_if = "Confirm::is_default")]
    pub confirm: Confirm,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

/// Which kinds of change the chat proposes for the user to apply (`true`) rather than making
/// itself (`false`) (ai D22). A key left out means `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirm {
    /// `propose_edit` and `propose_create`.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub notes: bool,
    /// Archiving and unarchiving.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub archive: bool,
    /// `propose_delete`, which moves a note to the trash; nothing ever purges (ai D21).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub delete: bool,
    /// `propose_type_change`.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub types: bool,
}

impl Default for Confirm {
    fn default() -> Confirm {
        Confirm {
            notes: true,
            archive: true,
            delete: true,
            types: true,
        }
    }
}

impl Confirm {
    fn is_default(&self) -> bool {
        *self == Confirm::default()
    }

    /// Every key filled, as `GET /api/integrations/ai` answers it.
    pub fn to_json(self) -> Value {
        serde_json::json!({
            "notes": self.notes,
            "archive": self.archive,
            "delete": self.delete,
            "types": self.types,
        })
    }
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

    /// Whether the chat system prompt is the user's own rather than the shipped default.
    pub fn system_prompt_custom(&self) -> bool {
        self.system_prompt
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty())
    }

    /// The chat system prompt in use: the user's own, else [`crate::CHAT_PROMPT`] (ai D17).
    pub fn chat_prompt(&self) -> &str {
        match self.system_prompt.as_deref() {
            Some(p) if !p.trim().is_empty() => p,
            _ => crate::CHAT_PROMPT,
        }
    }

    /// Set the chat system prompt; an empty one resets it to the default. Errors with `invalid`
    /// over [`SYSTEM_PROMPT_MAX`] characters.
    pub fn set_system_prompt(&mut self, prompt: &str) -> Result<(), AiError> {
        let n = prompt.chars().count();
        if n > SYSTEM_PROMPT_MAX {
            return Err(AiError::invalid(format!(
                "the system prompt is {n} characters; the most it can be is {SYSTEM_PROMPT_MAX}"
            )));
        }
        self.system_prompt = Some(prompt.to_owned()).filter(|p| !p.trim().is_empty());
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_keys_left_out_mean_true_and_only_offs_are_stored() {
        let s: Settings = serde_json::from_str(r#"{"confirm":{"delete":false}}"#).unwrap();
        assert_eq!(
            s.confirm,
            Confirm {
                delete: false,
                ..Confirm::default()
            }
        );
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains(r#""confirm":{"delete":false}"#), "{text}");
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.confirm, Confirm::default());
        assert!(!serde_json::to_string(&s).unwrap().contains("confirm"));
    }
}
