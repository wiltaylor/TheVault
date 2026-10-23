//! The `ai_settings` secret (ai D7): the default provider, the default model per provider, and
//! the base URLs the clients call: the API URLs of Settings > AI (ai D24), which tests point at
//! the testkit fakes.

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{AiError, ErrorKind};
use crate::types::Provider;

/// The secret holding [`Settings`].
pub const SETTINGS: &str = "ai_settings";

/// Anthropic's API, which `/v1/messages` and `/v1/models` hang off.
pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com";

/// OpenAI's API, for an API key and a ChatGPT plan token alike (ai D1). The ChatGPT plan always
/// uses it, whatever API URL the OpenAI card names (ai D24).
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

/// Base URLs in place of the real hosts. Absent or empty means the real one.
///
/// `anthropic` and `openai` are the API URLs Settings > AI edits (ai D24); tests point them at
/// the testkit fakes. The `anthropic` one takes `/v1/messages` and `/v1/models`, the `openai` one
/// `/v1/responses` or `/v1/chat/completions` (as `openai_api` says, ai D25) and `/v1/models`. A URL whose path ends in a version segment such as `/v1` or
/// `/v4` is the API root as it is; any other gets `/v1` appended (see [`api_url`]). `chatgpt` is the ChatGPT plan's base, a test seam that is not in
/// the UI: the plan never follows `openai`. `chatgpt_auth` takes `/api/accounts/oauth/token` and
/// `/.well-known/openid-configuration`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_auth: Option<String>,
    /// The API the OpenAI card's custom URL speaks (ai D25). Absent means Responses. Read only
    /// with a custom OpenAI URL: the official API and the ChatGPT plan always use Responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api: Option<OpenAiApi>,
}

/// The API an OpenAI-compatible service speaks (ai D25): `responses` (`POST /v1/responses`) or
/// `chat_completions` (`POST /v1/chat/completions`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiApi {
    #[default]
    Responses,
    ChatCompletions,
}

impl OpenAiApi {
    /// `responses` or `chat_completions`, as stored and as the routes answer it.
    pub fn as_str(self) -> &'static str {
        match self {
            OpenAiApi::Responses => "responses",
            OpenAiApi::ChatCompletions => "chat_completions",
        }
    }

    /// `Responses API` or `Chat Completions API`, as the user reads it.
    pub fn label(self) -> &'static str {
        match self {
            OpenAiApi::Responses => "Responses API",
            OpenAiApi::ChatCompletions => "Chat Completions API",
        }
    }

    /// The other one, which Test tries when a service lacks this one.
    pub fn other(self) -> OpenAiApi {
        match self {
            OpenAiApi::Responses => OpenAiApi::ChatCompletions,
            OpenAiApi::ChatCompletions => OpenAiApi::Responses,
        }
    }
}

impl std::str::FromStr for OpenAiApi {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "responses" => Ok(OpenAiApi::Responses),
            "chat_completions" => Ok(OpenAiApi::ChatCompletions),
            other => Err(format!(
                "unknown OpenAI API type {other:?}: expected responses or chat_completions"
            )),
        }
    }
}

impl Endpoints {
    /// The base URL `provider`'s inference and model calls go to: the custom API URL, else the
    /// real host. [`api_url`] joins a call's `/v1/...` path to it. The ChatGPT plan uses `chatgpt` or
    /// [`OPENAI_BASE`], never the OpenAI API URL (ai D24).
    pub fn api_base(&self, provider: Provider) -> String {
        let (set, real) = match provider {
            Provider::Anthropic => (&self.anthropic, ANTHROPIC_BASE),
            Provider::OpenAi => (&self.openai, OPENAI_BASE),
            Provider::ChatGpt => (&self.chatgpt, OPENAI_BASE),
        };
        match set.as_deref().map(trimmed).filter(|s| !s.is_empty()) {
            Some(s) => s.to_owned(),
            None => real.to_owned(),
        }
    }

    /// The API URL set for `provider`, as stored. Only Anthropic and OpenAI have one (ai D24).
    pub fn url(&self, provider: Provider) -> Option<&str> {
        match provider {
            Provider::Anthropic => self.anthropic.as_deref(),
            Provider::OpenAi => self.openai.as_deref(),
            Provider::ChatGpt => None,
        }
        .map(trimmed)
        .filter(|s| !s.is_empty())
    }

    /// The host, with its port when the URL names one, of `provider`'s custom API URL, for the
    /// trail line (ai D24). `None` for the official API and for the ChatGPT plan.
    pub fn custom_host(&self, provider: Provider) -> Option<String> {
        let url = Url::parse(self.url(provider)?).ok()?;
        let host = url.host_str()?;
        Some(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        })
    }

    /// Set `provider`'s API URL, checked by [`check_url`], or clear it with `None` or an empty
    /// string. The ChatGPT plan takes none.
    pub fn set_url(&mut self, provider: Provider, url: Option<&str>) -> Result<(), AiError> {
        let slot = match provider {
            Provider::Anthropic => &mut self.anthropic,
            Provider::OpenAi => &mut self.openai,
            Provider::ChatGpt => {
                return Err(AiError::invalid(
                    "the ChatGPT plan always uses OpenAI's own API: it takes no API URL",
                ));
            }
        };
        *slot = match url.map(trimmed).filter(|u| !u.is_empty()) {
            Some(u) => Some(check_url(u)?),
            None => None,
        };
        Ok(())
    }

    /// The API the OpenAI provider's calls speak (ai D25): the stored type with a custom OpenAI
    /// URL, and always Responses without one.
    pub fn openai_api(&self) -> OpenAiApi {
        match self.url(Provider::OpenAi) {
            Some(_) => self.openai_api.unwrap_or_default(),
            None => OpenAiApi::Responses,
        }
    }

    /// The base URL of the ChatGPT sign-in server.
    pub fn chatgpt_auth_base(&self) -> String {
        match self
            .chatgpt_auth
            .as_deref()
            .map(trimmed)
            .filter(|s| !s.is_empty())
        {
            Some(s) => s.to_owned(),
            None => CHATGPT_AUTH_BASE.to_owned(),
        }
    }
}

/// `s` trimmed and without trailing slashes.
fn trimmed(s: &str) -> &str {
    s.trim().trim_end_matches('/')
}

/// `url` as the settings keep it: trimmed and without trailing slashes, checked to be an `http`
/// or `https` URL with a host and no user name, password, query or fragment (ai D24). `http` is
/// allowed, for servers on the local network. A refusal is an `invalid` error saying why.
pub fn check_url(url: &str) -> Result<String, AiError> {
    let url = trimmed(url);
    let refuse = |why: &str| AiError::invalid(format!("The API URL {why}."));
    let parsed = Url::parse(url).map_err(|_| {
        refuse("is not a URL: give the whole address, such as https://api.example.com/v1")
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(refuse("must start with http:// or https://"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(refuse(
            "must not hold a user name or password: put the key in the API key field",
        ));
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(refuse("names no host"));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(refuse("must not have a query (?) or a fragment (#)"));
    }
    Ok(url.to_owned())
}

/// The URL of a call: `path` (such as `/v1/responses`) under `base`. A base whose path already
/// ends in a version segment (`/v1`, `/v4`) is the API root, so the path's own `/v1` is dropped:
/// `https://openrouter.ai/api/v1` and `https://openrouter.ai/api` both reach
/// `https://openrouter.ai/api/v1/responses`, and `https://api.z.ai/api/coding/paas/v4` reaches
/// `https://api.z.ai/api/coding/paas/v4/responses` (ai D24).
pub fn api_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let versioned = base
        .rsplit('/')
        .next()
        .and_then(|last| last.strip_prefix('v'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    match path.strip_prefix("/v1") {
        Some(rest)
            if versioned && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('?')) =>
        {
            format!("{base}{rest}")
        }
        _ => format!("{base}{path}"),
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
    /// Lifts the chat fetcher's address rule, so `fetch_url` and `download_asset` reach
    /// addresses on the local network (ai D27). An escape hatch with no UI; absent means false.
    #[serde(default, skip_serializing_if = "is_no")]
    pub fetch_private: bool,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

fn is_no(b: &bool) -> bool {
    !*b
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
    /// `download_asset`, which stores a file from the web as an asset (ai D27).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub downloads: bool,
}

impl Default for Confirm {
    fn default() -> Confirm {
        Confirm {
            notes: true,
            archive: true,
            delete: true,
            types: true,
            downloads: true,
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
            "downloads": self.downloads,
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
