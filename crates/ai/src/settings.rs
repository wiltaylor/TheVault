//! The `ai_settings` secret (ai D7): the global AI settings that belong to no account (accounts
//! A4), and the hidden test seams for the ChatGPT plan's hosts. The accounts themselves are
//! `ai_accounts` ([`crate::Accounts`]). The provider-keyed settings of before accounts
//! (`default_provider`, `models`, `endpoints.anthropic`, `endpoints.openai`,
//! `endpoints.openai_api` and the per-provider `context` windows) are read only by the migration
//! that made accounts of them (accounts A9); they are kept through a load and save.

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{AiError, ErrorKind};

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

/// The hidden test seams in place of the ChatGPT plan's real hosts. Absent or empty means the
/// real one. `chatgpt` is the plan's API base, which takes `/v1/responses` and `/v1/models`;
/// `chatgpt_auth` the sign-in server's, which takes `/api/accounts/oauth/token` and
/// `/.well-known/openid-configuration`. Neither is in the UI: tests point them at the testkit
/// fakes. Keys this build does not know, such as the API URLs of before accounts, are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_auth: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Endpoints {
    /// The base URL of the ChatGPT plan's calls: the seam, else [`OPENAI_BASE`].
    pub fn chatgpt_base(&self) -> String {
        match self
            .chatgpt
            .as_deref()
            .map(trimmed)
            .filter(|s| !s.is_empty())
        {
            Some(s) => s.to_owned(),
            None => OPENAI_BASE.to_owned(),
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
    #[serde(default, skip_serializing_if = "is_default")]
    pub endpoints: Endpoints,
    /// The chat panel's system prompt (ai D17). Absent or empty means [`crate::CHAT_PROMPT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Which kinds of chat change wait for the user (ai D22).
    #[serde(default, skip_serializing_if = "Confirm::is_default")]
    pub confirm: Confirm,
    /// Which internet tools the chat is given (ai D28).
    #[serde(default, skip_serializing_if = "Web::is_default")]
    pub web: Web,
    /// Lifts the chat fetcher's address rule, so `fetch_url` and `download_asset` reach
    /// addresses on the local network (ai D27). An escape hatch with no UI; absent means false.
    #[serde(default, skip_serializing_if = "is_no")]
    pub fetch_private: bool,
    /// How much a chat may send before it is compacted.
    #[serde(default, skip_serializing_if = "Context::is_default")]
    pub context: Context,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// When a chat thread is compacted. Each account has its own context window
/// ([`crate::Account::window`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Compact a thread before a turn once its last request filled [`Context::compact_at`] of
    /// the window.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub auto_compact: bool,
    /// The share of the window, in percent, that starts a compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_at: Option<u8>,
}

impl Default for Context {
    fn default() -> Context {
        Context {
            auto_compact: true,
            compact_at: None,
        }
    }
}

impl Context {
    /// The share of the window that starts a compaction when none is set.
    pub const COMPACT_AT: u8 = 80;
    /// The windows an account takes.
    pub const WINDOW_RANGE: std::ops::RangeInclusive<u64> = 4_000..=10_000_000;
    /// The shares the settings take.
    pub const COMPACT_RANGE: std::ops::RangeInclusive<u8> = 50..=95;

    fn is_default(&self) -> bool {
        *self == Context::default()
    }

    /// The share of the window, in percent, that starts a compaction.
    pub fn compact_at(&self) -> u8 {
        self.compact_at.unwrap_or(Context::COMPACT_AT)
    }
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
    /// `propose_type_change` and `propose_type_delete`.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub types: bool,
    /// `download_asset`, which stores a file from the web as an asset (ai D27).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub downloads: bool,
    /// `propose_taxonomy_change` and `propose_taxonomy_delete` (ai D31).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub taxonomies: bool,
    /// `propose_agent_change` and `propose_agent_delete` (agents G20).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub agents: bool,
    /// The skill propose tools (skills S9).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub skills: bool,
}

impl Default for Confirm {
    fn default() -> Confirm {
        Confirm {
            notes: true,
            archive: true,
            delete: true,
            types: true,
            downloads: true,
            taxonomies: true,
            agents: true,
            skills: true,
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
            "taxonomies": self.taxonomies,
            "agents": self.agents,
            "skills": self.skills,
        })
    }
}

/// Which internet tools the chat is given (ai D28): a tool switched off is left out of the tool
/// list, so the model never sees it. A key left out means `true`. `search` still needs a Brave
/// Search key, and `download` still follows [`Confirm::downloads`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Web {
    /// `web_search` (ai D26).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub search: bool,
    /// `fetch_url` (ai D27).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub read: bool,
    /// `download_asset` (ai D27).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub download: bool,
}

impl Default for Web {
    fn default() -> Web {
        Web {
            search: true,
            read: true,
            download: true,
        }
    }
}

impl Web {
    fn is_default(&self) -> bool {
        *self == Web::default()
    }

    /// Every key filled, as `GET /api/integrations/ai` answers it.
    pub fn to_json(self) -> Value {
        serde_json::json!({
            "search": self.search,
            "read": self.read,
            "download": self.download,
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
