//! A fresh `state.db` with a key, its `ai_settings` pointed at the testkit fakes.

#![allow(dead_code)]

use std::sync::Arc;

use ai::{Credential, Credentials, Endpoints, Event, Provider, Settings};
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
                anthropic: Some(anthropic.base_url()),
                openai: Some(openai.base_url()),
                chatgpt: Some(openai.base_url()),
                chatgpt_auth: Some(siwc.base_url()),
            },
            ..Settings::default()
        };
        settings.save(&state).expect("save settings");
        state
            .set_secret(ai::ANTHROPIC_API_KEY, anthropic.api_key())
            .unwrap();
        state
            .set_secret(ai::OPENAI_API_KEY, openai.api_key())
            .unwrap();
        Env {
            _dir: dir,
            state: Arc::new(state),
            anthropic,
            openai,
            siwc,
        }
    }

    pub fn cred(&self, provider: Provider) -> Credential {
        Credentials::resolve(&self.state, provider).expect("resolve credential")
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
            .set_secret(ai::CHATGPT, &auth.to_string())
            .unwrap();
    }

    pub fn chatgpt_secret(&self) -> Value {
        serde_json::from_str(&self.state.get_secret(ai::CHATGPT).unwrap().unwrap()).unwrap()
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
