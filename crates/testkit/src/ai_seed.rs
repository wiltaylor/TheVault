//! The secrets that set up AI accounts at the fakes (accounts A12): `ai_accounts`, each
//! account's key or ChatGPT sign-in, and the `ai_settings` the ChatGPT plan's test seam and
//! `fetch_private` need. Tests store them however they store secrets: through `PUT
//! /api/secrets/{name}` on a running server, or into a `state.db` they opened.
//!
//! ```no_run
//! use testkit::{AiSeed, FakeAnthropic, TestAccount};
//!
//! let fake = FakeAnthropic::start();
//! let seed = AiSeed::new().account(TestAccount::anthropic(&fake).model("claude-opus-5"));
//! for (name, value) in seed.secrets() {
//!     // PUT /api/secrets/{name} {"value": value}
//! }
//! ```

use serde_json::{Map, Value, json};

use crate::{FakeAnthropic, FakeOpenAi};

/// One account to seed. Its id is `acct_<name>`, the name lowercased with each run of other
/// characters as `_`, so a test can name it in a request.
#[derive(Debug, Clone)]
pub struct TestAccount {
    name: String,
    kind: &'static str,
    base_url: Option<String>,
    model: Option<String>,
    key: Option<String>,
    headers: Vec<(String, String)>,
    window: Option<u64>,
    /// A `chatgpt` account's sign-in, and the plan's base URL for the test seam.
    chatgpt: Option<(Value, String)>,
}

impl TestAccount {
    fn new(name: &str, kind: &'static str) -> TestAccount {
        TestAccount {
            name: name.to_owned(),
            kind,
            base_url: None,
            model: None,
            key: None,
            headers: Vec::new(),
            window: None,
            chatgpt: None,
        }
    }

    /// `Anthropic`, kind `anthropic`, at `fake` with its key.
    pub fn anthropic(fake: &FakeAnthropic) -> TestAccount {
        TestAccount {
            base_url: Some(fake.base_url()),
            key: Some(fake.api_key().to_owned()),
            ..TestAccount::new("Anthropic", "anthropic")
        }
    }

    /// `OpenAI`, kind `openai` (the Responses API), at `fake` with its key.
    pub fn openai(fake: &FakeOpenAi) -> TestAccount {
        TestAccount {
            base_url: Some(fake.base_url()),
            key: Some(fake.api_key().to_owned()),
            ..TestAccount::new("OpenAI", "openai")
        }
    }

    /// `Z.ai`, kind `openai_compatible` (Chat Completions), at `fake` with its key.
    pub fn openai_compatible(fake: &FakeOpenAi) -> TestAccount {
        TestAccount {
            base_url: Some(fake.base_url()),
            key: Some(fake.api_key().to_owned()),
            ..TestAccount::new("Z.ai", "openai_compatible")
        }
    }

    /// `ChatGPT`, kind `chatgpt`, signed in with access token `access`, which is not the fake's
    /// API key, so the fake treats it as a plan token. The token expires in an hour, so no
    /// refresh runs.
    pub fn chatgpt(fake: &FakeOpenAi, access: &str) -> TestAccount {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let auth = json!({
            "client_id": "oaiapp_test",
            "host_id": "urn:uuid:00000000-0000-4000-8000-000000000000",
            "access_token": access,
            "refresh_token": "test-refresh-token",
            "expires_ms": now + 60 * 60 * 1000,
            "account": "me@example.com",
        });
        TestAccount {
            chatgpt: Some((auth, fake.base_url())),
            ..TestAccount::new("ChatGPT", "chatgpt")
        }
    }

    /// The same account under another name.
    pub fn named(mut self, name: &str) -> TestAccount {
        self.name = name.to_owned();
        self
    }

    /// Its default model.
    pub fn model(mut self, model: &str) -> TestAccount {
        self.model = Some(model.to_owned());
        self
    }

    /// Its base URL in place of the fake's.
    pub fn base_url(mut self, url: &str) -> TestAccount {
        self.base_url = Some(url.to_owned());
        self
    }

    /// No API key: an account with its own base URL then sends no auth header.
    pub fn keyless(mut self) -> TestAccount {
        self.key = None;
        self
    }

    /// An extra header (accounts A3), for an `openai_compatible` account.
    pub fn header(mut self, name: &str, value: &str) -> TestAccount {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Its own context window.
    pub fn window(mut self, tokens: u64) -> TestAccount {
        self.window = Some(tokens);
        self
    }

    /// `acct_<name>`, the id it is seeded with.
    pub fn id(&self) -> String {
        account_id(&self.name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The id [`TestAccount`] `name` is seeded with: `acct_` and the name lowercased, each run of
/// other characters as `_`.
pub fn account_id(name: &str) -> String {
    let mut id = String::from("acct_");
    let mut gap = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
            gap = false;
        } else if !gap {
            id.push('_');
            gap = true;
        }
    }
    id
}

/// Accounts and the settings beside them, as secrets to store. The first account is the
/// default unless [`AiSeed::default_to`] says otherwise.
#[derive(Debug, Clone, Default)]
pub struct AiSeed {
    accounts: Vec<TestAccount>,
    default: Option<String>,
    settings: Map<String, Value>,
    legacy: Map<String, Value>,
}

impl AiSeed {
    pub fn new() -> AiSeed {
        AiSeed::default()
    }

    /// One account after those already added.
    pub fn account(mut self, a: TestAccount) -> AiSeed {
        self.accounts.push(a);
        self
    }

    /// Make the account named `name` the default.
    pub fn default_to(mut self, name: &str) -> AiSeed {
        self.default = Some(account_id(name));
        self
    }

    /// Map a provider name of before accounts (`anthropic`, `openai` or `chatgpt`) to the
    /// account named `name`, as the migration does (accounts A9).
    pub fn legacy(mut self, provider: &str, name: &str) -> AiSeed {
        self.legacy
            .insert(provider.to_owned(), json!(account_id(name)));
        self
    }

    /// Set a top-level key of `ai_settings`, such as `fetch_private`.
    pub fn setting(mut self, key: &str, value: Value) -> AiSeed {
        self.settings.insert(key.to_owned(), value);
        self
    }

    /// `(secret name, value)` for `ai_accounts`, each key and sign-in, and `ai_settings` when
    /// a setting or a ChatGPT account needs it.
    pub fn secrets(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut records = Vec::new();
        let mut settings = self.settings.clone();
        for a in &self.accounts {
            let id = a.id();
            let headers: Vec<Value> = a.headers.iter().map(|(n, v)| json!([n, v])).collect();
            records.push(json!({
                "id": id,
                "name": a.name,
                "kind": a.kind,
                "base_url": a.base_url.as_ref().filter(|_| a.kind != "chatgpt"),
                "model": a.model,
                "headers": headers,
                "context": a.window.map(|w| json!({ "window": w })),
            }));
            if let Some(k) = &a.key {
                out.push((format!("ai_account_key_{id}"), k.clone()));
            }
            if let Some((auth, base)) = &a.chatgpt {
                out.push((format!("ai_account_chatgpt_{id}"), auth.to_string()));
                let endpoints = settings.entry("endpoints").or_insert_with(|| json!({}));
                endpoints["chatgpt"] = json!(base);
            }
        }
        let default = self
            .default
            .clone()
            .or_else(|| self.accounts.first().map(TestAccount::id));
        let mut table = json!({ "accounts": records, "default": default });
        if !self.legacy.is_empty() {
            table["legacy"] = Value::Object(self.legacy.clone());
        }
        out.push(("ai_accounts".to_owned(), table.to_string()));
        if !settings.is_empty() {
            out.push((
                "ai_settings".to_owned(),
                Value::Object(settings).to_string(),
            ));
        }
        out
    }

    /// Store [`AiSeed::secrets`] in `state`.
    pub fn store(&self, state: &state::State) {
        for (name, value) in self.secrets() {
            state
                .set_secret(&name, &value)
                .expect("store a seeded secret");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_from_names() {
        assert_eq!(account_id("Anthropic"), "acct_anthropic");
        assert_eq!(account_id("Z.ai  work"), "acct_z_ai_work");
    }
}
