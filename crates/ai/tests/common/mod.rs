//! A fresh `state.db` with a key and three accounts pointed at the testkit fakes: `Anthropic`
//! (anthropic), `OpenAI` (openai) and `ChatGPT` (chatgpt, signed in by [`Env::sign_in`]).

#![allow(dead_code)]

use std::sync::Arc;

use ai::{
    Account, AccountInput, Accounts, Credential, Credentials, Endpoints, Event, Kind, Settings,
};
use serde_json::{Value, json};
use state::{SecretKey, State};
use tempfile::TempDir;
use testkit::{FakeAnthropic, FakeOpenAi, FakeSiwc};

pub struct Env {
    _dir: TempDir,
    pub state: Arc<State>,
    pub anthropic: FakeAnthropic,
    pub openai: FakeOpenAi,
    pub siwc: FakeSiwc,
}

/// Add an account named `name` of `kind` at `url` to the stored table, with `key` when given.
pub fn add_account(
    state: &State,
    name: &str,
    kind: Kind,
    url: Option<&str>,
    key: Option<&str>,
) -> Account {
    let mut all = Accounts::load(state).unwrap();
    let a = all
        .add(AccountInput {
            name: Some(name.to_owned()),
            kind: Some(kind),
            base_url: Some(url.map(str::to_owned)),
            ..AccountInput::default()
        })
        .unwrap()
        .clone();
    all.save(state).unwrap();
    if let Some(k) = key {
        state.set_secret(&ai::key_secret(&a.id), k).unwrap();
    }
    a
}

impl Env {
    pub fn new() -> Env {
        Self::with_siwc(FakeSiwc::start())
    }

    pub fn with_siwc(siwc: FakeSiwc) -> Env {
        let dir = tempfile::tempdir().expect("temp dir");
        let state = State::open(&dir.path().join("state.db"), Some(SecretKey::generate()))
            .expect("open state");
        let anthropic = FakeAnthropic::start();
        let openai = FakeOpenAi::start();
        let settings = Settings {
            endpoints: Endpoints {
                chatgpt: Some(openai.base_url()),
                chatgpt_auth: Some(siwc.base_url()),
                ..Endpoints::default()
            },
            ..Settings::default()
        };
        settings.save(&state).expect("save settings");
        let a = anthropic.base_url();
        add_account(
            &state,
            "Anthropic",
            Kind::Anthropic,
            Some(&a),
            Some(anthropic.api_key()),
        );
        let o = openai.base_url();
        add_account(
            &state,
            "OpenAI",
            Kind::OpenAi,
            Some(&o),
            Some(openai.api_key()),
        );
        add_account(&state, "ChatGPT", Kind::ChatGpt, None, None);
        Env {
            _dir: dir,
            state: Arc::new(state),
            anthropic,
            openai,
            siwc,
        }
    }

    /// The first account of `kind`.
    pub fn account(&self, kind: Kind) -> Account {
        Accounts::load(&self.state)
            .unwrap()
            .accounts
            .into_iter()
            .find(|a| a.kind == kind)
            .expect("an account of the kind")
    }

    pub fn cred(&self, kind: Kind) -> Credential {
        Credentials::resolve(&self.state, &self.account(kind)).expect("resolve credential")
    }

    /// The ChatGPT account's secret.
    pub fn chatgpt_name(&self) -> String {
        ai::chatgpt_secret(&self.account(Kind::ChatGpt).id)
    }

    /// Store a ChatGPT sign-in whose access token expires `expires_in_ms` from now, with the
    /// refresh token the fake accepts.
    pub fn sign_in(&self, access: &str, expires_in_ms: i64) {
        let auth = json!({
            "client_id": self.siwc.client_id(),
            "host_id": "urn:uuid:00000000-0000-4000-8000-000000000000",
            "access_token": access,
            "refresh_token": self.siwc.refresh_token(),
            "expires_ms": now_ms() + expires_in_ms,
            "id_token": "eyJhbGciOiJSUzI1NiJ9.e30.c2ln",
            "account": "me@example.com",
        });
        self.state
            .set_secret(&self.chatgpt_name(), &auth.to_string())
            .unwrap();
    }

    pub fn chatgpt_secret(&self) -> Value {
        serde_json::from_str(
            &self
                .state
                .get_secret(&self.chatgpt_name())
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Every event of a stream, until it ends or fails.
pub async fn events(stream: &mut ai::EventStream) -> (Vec<Event>, Option<ai::AiError>) {
    let mut out = Vec::new();
    while let Some(e) = stream.next().await {
        match e {
            Ok(e) => out.push(e),
            Err(e) => return (out, Some(e)),
        }
    }
    (out, None)
}

pub fn text_of(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

/// A strict tool schema.
pub fn get_note_tool() -> ai::ToolSpec {
    ai::ToolSpec {
        name: "get_note".to_owned(),
        description: "Read one note by id.".to_owned(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["id", "fields"],
            "properties": {
                "id": {"type": "string"},
                "fields": {"type": "array", "items": {"type": "string"}}
            }
        }),
    }
}
