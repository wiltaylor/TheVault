//! Named AI accounts (accounts A1 to A5), the table Settings > AI edits in place of the three
//! fixed providers.
//!
//! - `ai_accounts` holds [`Accounts`]: the account records in the table's order, the default
//!   account's id, and `legacy`, the provider names of before accounts mapped to the accounts the
//!   migration made for them (accounts A9).
//! - `ai_account_key_<id>` ([`key_secret`]) holds an account's API key.
//! - `ai_account_chatgpt_<id>` ([`chatgpt_secret`]) holds a `chatgpt` account's
//!   [`crate::ChatGptAuth`].
//! - `ai_account_usage_<id>` ([`usage_secret`]) holds the last [`PlanUsage`] a `chatgpt` account's
//!   plan reported (accounts A13).
//!
//! All four are state.db secrets, encrypted with the vault key. No route ever answers a key or a
//! token.

use std::collections::BTreeMap;
use std::sync::Mutex;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::AiError;
use crate::settings::{ANTHROPIC_BASE, Context, Endpoints, OPENAI_BASE, check_url, state_error};
use crate::types::Kind;

/// The secret holding [`Accounts`].
pub const ACCOUNTS: &str = "ai_accounts";

/// The longest account name, in characters.
pub const NAME_MAX: usize = 60;

/// The most extra headers an `openai_compatible` account takes.
pub const HEADERS_MAX: usize = 20;

/// The header names an account's extra headers may not set (accounts A3).
pub const REFUSED_HEADERS: [&str; 4] = ["authorization", "content-type", "content-length", "host"];

/// One change to `ai_accounts` at a time in the whole process: every change is a read, a
/// modify and a write ([`Accounts::change`]), and two at once would lose one.
static WRITE: Mutex<()> = Mutex::new(());

/// The secret holding account `id`'s API key. Secret names are lower case, so the ULID is too.
pub fn key_secret(id: &str) -> String {
    format!("ai_account_key_{}", id.to_ascii_lowercase())
}

/// The secret holding account `id`'s ChatGPT sign-in.
pub fn chatgpt_secret(id: &str) -> String {
    format!("ai_account_chatgpt_{}", id.to_ascii_lowercase())
}

/// The secret holding the last usage reading of account `id`'s plan.
pub fn usage_secret(id: &str) -> String {
    format!("ai_account_usage_{}", id.to_ascii_lowercase())
}

/// An account's own context window, when it sets one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountContext {
    pub window: u64,
}

/// One account (accounts A2). Keys this build does not know are kept through a load and save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// A ULID, stable across renames.
    pub id: String,
    /// Unique among the accounts, ignoring case.
    pub name: String,
    pub kind: Kind,
    /// `None` is the kind's own ([`default_base`]). Always set for `openai_compatible`; never for
    /// `chatgpt`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// The default model; `None` is the kind's default.
    #[serde(default)]
    pub model: Option<String>,
    /// Extra headers sent on every request, `openai_compatible` only (accounts A3).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// `None` is the kind's default window ([`default_window`]).
    #[serde(default)]
    pub context: Option<AccountContext>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Account {
    /// The model calls use when none is asked for, if the account names one.
    pub fn model(&self) -> Option<&str> {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
    }

    /// The context window in tokens: the account's own, else its kind's default.
    pub fn window(&self) -> u64 {
        self.context
            .map(|c| c.window)
            .unwrap_or_else(|| default_window(self.kind))
    }

    /// The base URL calls go to: the account's own, else the kind's. The ChatGPT plan's is
    /// fixed, apart from the hidden test seam `ai_settings.endpoints.chatgpt`.
    pub fn api_base(&self, seam: &Endpoints) -> String {
        if self.kind == Kind::ChatGpt {
            return seam.chatgpt_base();
        }
        match self.url() {
            Some(u) => u.to_owned(),
            None => default_base(self.kind).unwrap_or_default().to_owned(),
        }
    }

    /// The account's own base URL, as stored. `chatgpt` has none.
    pub fn url(&self) -> Option<&str> {
        if self.kind == Kind::ChatGpt {
            return None;
        }
        self.base_url
            .as_deref()
            .map(|s| s.trim().trim_end_matches('/'))
            .filter(|s| !s.is_empty())
    }

    /// The host, with its port when the URL names one, of the account's own base URL, for the
    /// trail line and the compatibility errors (ai D24). `None` for the kind's own host.
    pub fn custom_host(&self) -> Option<String> {
        let url = Url::parse(self.url()?).ok()?;
        let host = url.host_str()?;
        Some(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        })
    }
}

/// The base URL a kind's calls go to when the account sets none. `openai_compatible` has none:
/// its URL is required.
pub fn default_base(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Anthropic => Some(ANTHROPIC_BASE),
        Kind::OpenAi | Kind::ChatGpt => Some(OPENAI_BASE),
        Kind::OpenAiCompatible => None,
    }
}

/// The context window of a kind's account that sets none (accounts A2).
pub fn default_window(kind: Kind) -> u64 {
    match kind {
        Kind::Anthropic => 200_000,
        Kind::OpenAi => 400_000,
        Kind::OpenAiCompatible => 128_000,
        Kind::ChatGpt => 272_000,
    }
}

/// The `ai_accounts` secret (accounts A4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Accounts {
    /// In the table's order.
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// The default account's id.
    #[serde(default)]
    pub default: Option<String>,
    /// `anthropic`, `openai` and `chatgpt` mapped to the id of the account the migration made
    /// for each (accounts A9), so an agent file or a saved turn naming a provider still resolves.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy: BTreeMap<String, String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// What a new or changed account says, before it is checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountInput {
    pub name: Option<String>,
    pub kind: Option<Kind>,
    /// `Some(None)` clears it.
    pub base_url: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub headers: Option<Vec<(String, String)>>,
    pub context: Option<Option<AccountContext>>,
}

impl Accounts {
    /// The stored table, or an empty one when there is none.
    pub fn load(state: &state::State) -> Result<Accounts, AiError> {
        Ok(match state.get_secret(ACCOUNTS).map_err(state_error)? {
            Some(text) => serde_json::from_str(&text).unwrap_or_default(),
            None => Accounts::default(),
        })
    }

    pub fn save(&self, state: &state::State) -> Result<(), AiError> {
        let text = serde_json::to_string(self).map_err(|e| AiError::invalid(e.to_string()))?;
        state.set_secret(ACCOUNTS, &text).map_err(state_error)
    }

    /// Change the stored table with `f`, as one read, modify and write under a process-wide
    /// lock, so changes made at once are all kept. Nothing is saved when `f` fails. What `f`
    /// does beside the table, such as storing a key or rewriting agent files, happens under the
    /// lock too. `err` turns a load or save failure into the caller's error.
    pub fn change<T, E>(
        state: &state::State,
        err: impl Fn(AiError) -> E,
        f: impl FnOnce(&mut Accounts) -> Result<T, E>,
    ) -> Result<T, E> {
        let _one = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = Accounts::load(state).map_err(&err)?;
        let out = f(&mut all)?;
        all.save(state).map_err(&err)?;
        Ok(out)
    }

    /// The account with id `id`.
    pub fn get(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// The account `r` names: by id, else by name ignoring case, else by a provider name of
    /// before accounts ([`Accounts::legacy`]).
    pub fn find(&self, r: &str) -> Option<&Account> {
        let r = r.trim();
        if r.is_empty() {
            return None;
        }
        self.get(r)
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|a| a.name.to_lowercase() == r.to_lowercase())
            })
            .or_else(|| self.legacy.get(r).and_then(|id| self.get(id)))
    }

    /// The default account, else the first (accounts A5).
    pub fn default_account(&self) -> Option<&Account> {
        self.default
            .as_deref()
            .and_then(|id| self.get(id))
            .or_else(|| self.accounts.first())
    }

    /// The account a call uses (accounts A5): the one `asked` names, which must exist, else the
    /// one the agent names when it exists, else the default, else the first. None at all is
    /// `not_configured`.
    pub fn pick(&self, asked: Option<&str>, agent: Option<&str>) -> Result<&Account, AiError> {
        if let Some(r) = asked.map(str::trim).filter(|r| !r.is_empty()) {
            return self
                .find(r)
                .ok_or_else(|| AiError::invalid(format!("there is no AI account {r:?}")));
        }
        agent
            .and_then(|r| self.find(r))
            .or_else(|| self.default_account())
            .ok_or_else(no_account)
    }

    /// The legacy provider names that map to account `id`.
    pub fn legacy_names(&self, id: &str) -> Vec<String> {
        self.legacy
            .iter()
            .filter(|(_, to)| to.as_str() == id)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Add an account from `input`, which must name a name and a kind, checked as
    /// [`Accounts::update`] checks a change. The first account becomes the default.
    pub fn add(&mut self, input: AccountInput) -> Result<&Account, AiError> {
        let kind = input
            .kind
            .ok_or_else(|| AiError::invalid("an account needs a kind"))?;
        if input.name.is_none() {
            return Err(AiError::invalid("an account needs a name"));
        }
        let mut account = Account {
            id: ulid::Ulid::new().to_string(),
            name: String::new(),
            kind,
            base_url: None,
            model: None,
            headers: Vec::new(),
            context: None,
            other: Map::new(),
        };
        apply(self, &mut account, input)?;
        if self.accounts.is_empty() {
            self.default = Some(account.id.clone());
        }
        self.accounts.push(account);
        Ok(self.accounts.last().expect("just pushed"))
    }

    /// Change account `id` as `input` says. The kind cannot change. Returns the old name when
    /// the name changed.
    pub fn update(&mut self, id: &str, input: AccountInput) -> Result<Option<String>, AiError> {
        let at = self
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| AiError::invalid(format!("there is no AI account {id:?}")))?;
        let mut account = self.accounts[at].clone();
        if input.kind.is_some_and(|k| k != account.kind) {
            return Err(AiError::invalid(
                "an account's kind cannot change: add an account of the other kind instead",
            ));
        }
        let old = account.name.clone();
        apply(self, &mut account, input)?;
        let renamed = (account.name != old).then_some(old);
        self.accounts[at] = account;
        Ok(renamed)
    }

    /// Take account `id` out, with its legacy names. The default moves to the first account left.
    pub fn remove(&mut self, id: &str) -> Result<Account, AiError> {
        let at = self
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| AiError::invalid(format!("there is no AI account {id:?}")))?;
        let gone = self.accounts.remove(at);
        self.legacy.retain(|_, to| to != id);
        if self.default.as_deref() == Some(id) {
            self.default = self.accounts.first().map(|a| a.id.clone());
        }
        Ok(gone)
    }

    /// Put the accounts in the order of `ids`, which must name each account once.
    pub fn reorder(&mut self, ids: &[String]) -> Result<(), AiError> {
        let mut seen: Vec<&str> = ids.iter().map(String::as_str).collect();
        seen.sort_unstable();
        seen.dedup();
        let mut have: Vec<&str> = self.accounts.iter().map(|a| a.id.as_str()).collect();
        have.sort_unstable();
        if seen.len() != ids.len() || seen != have {
            return Err(AiError::invalid(
                "name every account once, in the new order",
            ));
        }
        let mut by_id: BTreeMap<String, Account> =
            self.accounts.drain(..).map(|a| (a.id.clone(), a)).collect();
        for id in ids {
            if let Some(a) = by_id.remove(id) {
                self.accounts.push(a);
            }
        }
        Ok(())
    }

    /// Make account `id` the default, or clear the default with `None`.
    pub fn set_default(&mut self, id: Option<&str>) -> Result<(), AiError> {
        match id.map(str::trim).filter(|i| !i.is_empty()) {
            Some(id) => {
                let id = self
                    .find(id)
                    .map(|a| a.id.clone())
                    .ok_or_else(|| AiError::invalid(format!("there is no AI account {id:?}")))?;
                self.default = Some(id);
            }
            None => self.default = None,
        }
        Ok(())
    }
}

/// The error when no account is set up at all (accounts A5).
pub fn no_account() -> AiError {
    AiError::not_configured("no AI account is set up: add one in Settings > AI")
}

/// Apply `input` to `account`, checking every field: the name (1 to [`NAME_MAX`] characters, no
/// line break, unique ignoring case), the base URL ([`check_url`]; required for
/// `openai_compatible`, refused for `chatgpt`), the headers ([`check_headers`]) and the window.
fn apply(all: &Accounts, account: &mut Account, input: AccountInput) -> Result<(), AiError> {
    if let Some(name) = input.name {
        let name = check_name(&name)?;
        if all
            .accounts
            .iter()
            .any(|a| a.id != account.id && a.name.to_lowercase() == name.to_lowercase())
        {
            return Err(AiError::invalid(format!(
                "there is already an account named {name:?}"
            )));
        }
        account.name = name;
    }
    if let Some(url) = input.base_url {
        account.base_url = match url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            Some(_) if account.kind == Kind::ChatGpt => {
                return Err(AiError::invalid(
                    "a ChatGPT subscription always uses OpenAI's own API: it takes no base URL",
                ));
            }
            Some(u) => Some(check_url(u)?),
            None => None,
        };
    }
    if account.kind == Kind::OpenAiCompatible && account.url().is_none() {
        return Err(AiError::invalid(
            "an OpenAI-compatible account needs its base URL, such as https://openrouter.ai/api/v1",
        ));
    }
    if let Some(model) = input.model {
        account.model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    }
    if let Some(headers) = input.headers {
        if account.kind != Kind::OpenAiCompatible && !headers.is_empty() {
            return Err(AiError::invalid(
                "only an OpenAI-compatible account takes extra headers",
            ));
        }
        account.headers = check_headers(&headers)?;
    }
    if let Some(context) = input.context {
        if let Some(c) = context
            && !Context::WINDOW_RANGE.contains(&c.window)
        {
            return Err(AiError::invalid(format!(
                "a context window is from {} to {} tokens",
                Context::WINDOW_RANGE.start(),
                Context::WINDOW_RANGE.end()
            )));
        }
        account.context = context;
    }
    Ok(())
}

/// `name` trimmed, when it is 1 to [`NAME_MAX`] characters with no line break.
pub fn check_name(name: &str) -> Result<String, AiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AiError::invalid("an account needs a name"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(AiError::invalid(format!(
            "an account name is at most {NAME_MAX} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(AiError::invalid(
            "an account name is one line, with no control characters",
        ));
    }
    Ok(name.to_owned())
}

/// The extra headers, trimmed, when every name is a valid HTTP header name that is not one of
/// [`REFUSED_HEADERS`] and every value is a valid header value (accounts A3).
pub fn check_headers(headers: &[(String, String)]) -> Result<Vec<(String, String)>, AiError> {
    if headers.len() > HEADERS_MAX {
        return Err(AiError::invalid(format!(
            "an account takes at most {HEADERS_MAX} extra headers"
        )));
    }
    let mut out = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let name = name.trim();
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(AiError::invalid(format!(
                "{name:?} is not a valid header name"
            )));
        }
        if REFUSED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(AiError::invalid(format!(
                "the {name} header is the app's own: an account cannot set it"
            )));
        }
        let value = value.trim();
        if reqwest::header::HeaderValue::from_str(value).is_err() {
            return Err(AiError::invalid(format!(
                "the value of the {name} header is not a valid header value"
            )));
        }
        out.push((name.to_owned(), value.to_owned()));
    }
    Ok(out)
}

/// One limit window of a plan's usage (accounts A13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    /// Such as `5-hour` or `Weekly`.
    pub label: String,
    /// 0 to 100.
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    /// When the window resets, in Unix milliseconds.
    pub resets_at_ms: Option<i64>,
}

/// The last usage a subscription reported (accounts A13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanUsage {
    /// When it was read, in Unix milliseconds.
    pub read_at_ms: i64,
    pub windows: Vec<UsageWindow>,
    #[serde(default)]
    pub plan: Option<String>,
}

impl PlanUsage {
    /// The usage a ChatGPT plan response reports in its headers, as the Codex CLI reads them
    /// (`codex-rs/codex-api/src/rate_limits.rs`): `x-codex-primary-used-percent` (a percent),
    /// `x-codex-primary-window-minutes` and `x-codex-primary-reset-at` (Unix seconds), and the
    /// `secondary` three. A window counts only when its used percent reads. `None` when no
    /// window does.
    pub fn from_headers(headers: &reqwest::header::HeaderMap, now_ms: i64) -> Option<PlanUsage> {
        let header = |n: &str| {
            headers
                .get(n)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let mut windows = Vec::new();
        for (which, fallback) in [("primary", "Short"), ("secondary", "Long")] {
            let Some(used) = header(&format!("x-codex-{which}-used-percent"))
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
            else {
                continue;
            };
            let minutes = header(&format!("x-codex-{which}-window-minutes"))
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|m| *m > 0);
            let resets = header(&format!("x-codex-{which}-reset-at"))
                .and_then(|v| v.parse::<i64>().ok())
                .map(|s| s.saturating_mul(1000));
            windows.push(UsageWindow {
                label: minutes.map_or_else(|| fallback.to_owned(), window_label),
                used_percent: used.clamp(0.0, 100.0),
                window_minutes: minutes,
                resets_at_ms: resets,
            });
        }
        (!windows.is_empty()).then_some(PlanUsage {
            read_at_ms: now_ms,
            windows,
            plan: None,
        })
    }

    /// The reading stored for account `id`, if any.
    pub fn load(state: &state::State, id: &str) -> Result<Option<PlanUsage>, AiError> {
        Ok(state
            .get_secret(&usage_secret(id))
            .map_err(state_error)?
            .and_then(|t| serde_json::from_str(&t).ok()))
    }

    /// Keep this reading as account `id`'s latest.
    pub fn save(&self, state: &state::State, id: &str) -> Result<(), AiError> {
        let text = serde_json::to_string(self).map_err(|e| AiError::invalid(e.to_string()))?;
        state
            .set_secret(&usage_secret(id), &text)
            .map_err(state_error)
    }
}

/// A window's label from its length: `Weekly`, `N-day`, `N-hour` or `N-minute`.
fn window_label(minutes: i64) -> String {
    const DAY: i64 = 24 * 60;
    if minutes == 7 * DAY {
        "Weekly".to_owned()
    } else if minutes % DAY == 0 {
        format!("{}-day", minutes / DAY)
    } else if minutes % 60 == 0 {
        format!("{}-hour", minutes / 60)
    } else {
        format!("{minutes}-minute")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, kind: Kind) -> AccountInput {
        AccountInput {
            name: Some(name.to_owned()),
            kind: Some(kind),
            ..AccountInput::default()
        }
    }

    #[test]
    fn names_are_unique_ignoring_case_and_the_first_is_the_default() {
        let mut all = Accounts::default();
        let id = all.add(input("Work", Kind::Anthropic)).unwrap().id.clone();
        assert_eq!(all.default.as_deref(), Some(id.as_str()));
        assert!(all.add(input("work", Kind::OpenAi)).is_err());
        assert!(all.add(input("", Kind::OpenAi)).is_err());
        assert!(all.add(input("a\nb", Kind::OpenAi)).is_err());
        assert_eq!(all.find("WORK").unwrap().id, id);
        assert_eq!(all.find(&id).unwrap().name, "Work");
    }

    #[test]
    fn openai_compatible_needs_a_url_and_only_it_takes_headers() {
        let mut all = Accounts::default();
        assert!(all.add(input("Router", Kind::OpenAiCompatible)).is_err());
        let ok = AccountInput {
            base_url: Some(Some("https://openrouter.ai/api/v1/".into())),
            headers: Some(vec![("X-Title".into(), " TheVault ".into())]),
            ..input("Router", Kind::OpenAiCompatible)
        };
        let a = all.add(ok).unwrap();
        assert_eq!(a.base_url.as_deref(), Some("https://openrouter.ai/api/v1"));
        assert_eq!(a.headers, vec![("X-Title".into(), "TheVault".into())]);
        let bad = AccountInput {
            headers: Some(vec![("X-Title".into(), "x".into())]),
            ..input("Claude", Kind::Anthropic)
        };
        assert!(all.add(bad).is_err());
        let chatgpt = AccountInput {
            base_url: Some(Some("https://example.com".into())),
            ..input("Plan", Kind::ChatGpt)
        };
        assert!(all.add(chatgpt).is_err());
    }

    #[test]
    fn refused_and_malformed_headers_are_errors() {
        for name in [
            "Authorization",
            "content-type",
            "Content-Length",
            "HOST",
            "bad name",
            "",
        ] {
            assert!(
                check_headers(&[(name.into(), "v".into())]).is_err(),
                "{name:?}"
            );
        }
        assert!(check_headers(&[("X-A".into(), "a\nb".into())]).is_err());
        assert!(check_headers(&[("HTTP-Referer".into(), "https://vault.example".into())]).is_ok());
    }

    #[test]
    fn pick_follows_the_request_then_the_agent_then_the_default() {
        let mut all = Accounts::default();
        assert_eq!(
            all.pick(None, None).unwrap_err().kind,
            crate::ErrorKind::NotConfigured
        );
        let a = all.add(input("A", Kind::Anthropic)).unwrap().id.clone();
        let b = all.add(input("B", Kind::OpenAi)).unwrap().id.clone();
        all.legacy.insert("openai".into(), b.clone());
        assert_eq!(all.pick(None, None).unwrap().id, a);
        assert_eq!(all.pick(None, Some("b")).unwrap().id, b);
        assert_eq!(all.pick(None, Some("openai")).unwrap().id, b);
        assert_eq!(all.pick(None, Some("Gone")).unwrap().id, a);
        assert_eq!(all.pick(Some(&b), Some("A")).unwrap().id, b);
        assert!(all.pick(Some("Gone"), None).is_err());
        all.set_default(Some("B")).unwrap();
        assert_eq!(all.pick(None, None).unwrap().id, b);
        all.remove(&b).unwrap();
        assert_eq!(all.default.as_deref(), Some(a.as_str()));
        assert!(all.legacy.is_empty());
    }

    #[test]
    fn reorder_names_every_account_once() {
        let mut all = Accounts::default();
        let a = all.add(input("A", Kind::Anthropic)).unwrap().id.clone();
        let b = all.add(input("B", Kind::OpenAi)).unwrap().id.clone();
        assert!(all.reorder(std::slice::from_ref(&b)).is_err());
        assert!(all.reorder(&[b.clone(), b.clone()]).is_err());
        all.reorder(&[b.clone(), a.clone()]).unwrap();
        assert_eq!(all.accounts[0].id, b);
    }

    #[test]
    fn plan_usage_reads_the_codex_headers() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("x-codex-primary-used-percent", "12.5".parse().unwrap());
        h.insert("x-codex-primary-window-minutes", "300".parse().unwrap());
        h.insert("x-codex-primary-reset-at", "1704069000".parse().unwrap());
        h.insert("x-codex-secondary-used-percent", "40".parse().unwrap());
        h.insert("x-codex-secondary-window-minutes", "10080".parse().unwrap());
        let u = PlanUsage::from_headers(&h, 5).unwrap();
        assert_eq!(u.read_at_ms, 5);
        assert_eq!(u.windows.len(), 2);
        assert_eq!(u.windows[0].label, "5-hour");
        assert_eq!(u.windows[0].used_percent, 12.5);
        assert_eq!(u.windows[0].resets_at_ms, Some(1_704_069_000_000));
        assert_eq!(u.windows[1].label, "Weekly");
        assert_eq!(u.windows[1].resets_at_ms, None);
        assert!(PlanUsage::from_headers(&reqwest::header::HeaderMap::new(), 0).is_none());
    }
}
