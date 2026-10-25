//! Step `0010_ai_accounts`, which makes named AI accounts of the three fixed providers (accounts
//! A9), and the start rewrite of agent files that name a provider (accounts A6).

use std::path::Path;

use serde_json::{Value, json};
use state::{SecretKey, State};
use store::Vault;
use writer::{UpgradeStep, Upkeep, Writer};

fn open(root: &Path, key: Option<SecretKey>) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), key).unwrap();
    Writer::open(vault, state).unwrap()
}

fn generation(root: &Path) -> u64 {
    std::fs::read_to_string(root.join(".vault/generation"))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0)
}

/// A started vault with a key, its accounts step not yet run, holding `secrets`.
fn vault(root: &Path, secrets: &[(&str, String)]) -> Writer {
    let w = open(root, Some(SecretKey::generate()));
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    for (name, value) in secrets {
        w.state().set_secret(name, value).unwrap();
    }
    w
}

/// Run step `0010_ai_accounts` again under a new id, as a vault that has not run it yet would
/// on its next start.
fn run_step(w: &Writer) -> Vec<String> {
    let step = writer::UPGRADE_STEPS
        .iter()
        .find(|s| s.id == "0010_ai_accounts")
        .unwrap();
    w.run_upgrade_steps(&[UpgradeStep {
        id: "test_0010_ai_accounts",
        run: step.run,
    }])
    .unwrap()
}

fn secret(w: &Writer, name: &str) -> Option<String> {
    w.state().get_secret(name).unwrap()
}

fn accounts(w: &Writer) -> Value {
    serde_json::from_str(&secret(w, "ai_accounts").expect("ai_accounts")).unwrap()
}

fn id_of(table: &Value, name: &str) -> String {
    table["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no account {name}: {table}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn settings(v: Value) -> (&'static str, String) {
    ("ai_settings", v.to_string())
}

#[test]
fn an_anthropic_key_alone_becomes_the_default_anthropic_account() {
    let dir = tempfile::tempdir().unwrap();
    let w = vault(
        dir.path(),
        &[
            ("ai_anthropic_api_key", "sk-ant-1".into()),
            settings(json!({
                "models": { "anthropic": "claude-opus-5" },
                "context": { "anthropic": 150000 },
            })),
        ],
    );
    assert_eq!(run_step(&w), ["test_0010_ai_accounts"]);
    let t = accounts(&w);
    assert_eq!(t["accounts"].as_array().unwrap().len(), 1, "{t}");
    let a = &t["accounts"][0];
    assert_eq!(a["name"], "Anthropic");
    assert_eq!(a["kind"], "anthropic");
    assert_eq!(a["base_url"], Value::Null);
    assert_eq!(a["model"], "claude-opus-5");
    assert_eq!(a["context"], json!({ "window": 150000 }));
    let id = a["id"].as_str().unwrap();
    assert_eq!(id.len(), 26, "a ULID: {id}");
    assert_eq!(t["default"], json!(id));
    assert_eq!(t["legacy"], json!({ "anthropic": id }));
    assert_eq!(
        secret(&w, &format!("ai_account_key_{}", id.to_ascii_lowercase())).as_deref(),
        Some("sk-ant-1")
    );
    assert_eq!(
        secret(&w, "ai_anthropic_api_key"),
        None,
        "the old key is gone"
    );
}

#[test]
fn an_openai_key_becomes_an_openai_responses_account() {
    let dir = tempfile::tempdir().unwrap();
    let w = vault(
        dir.path(),
        &[
            ("ai_openai_api_key", "sk-oa".into()),
            settings(json!({
                "models": { "openai": "gpt-6" },
                // Chat Completions is read only with a URL: the official API speaks Responses.
                "endpoints": { "openai_api": "chat_completions" },
            })),
        ],
    );
    run_step(&w);
    let t = accounts(&w);
    let a = &t["accounts"][0];
    assert_eq!(
        (a["name"].as_str(), a["kind"].as_str()),
        (Some("OpenAI"), Some("openai"))
    );
    assert_eq!(a["base_url"], Value::Null);
    assert_eq!(a["model"], "gpt-6");
    let id = id_of(&t, "OpenAI");
    assert_eq!(
        secret(&w, &format!("ai_account_key_{}", id.to_ascii_lowercase())).as_deref(),
        Some("sk-oa")
    );
    assert_eq!(secret(&w, "ai_openai_api_key"), None);
}

#[test]
fn an_openai_url_speaking_chat_completions_becomes_an_openai_compatible_account() {
    let dir = tempfile::tempdir().unwrap();
    let w = vault(
        dir.path(),
        &[settings(json!({
            "default_provider": "openai",
            "models": { "openai": "glm-5.3-flash" },
            "endpoints": { "openai": "https://api.z.ai/api/paas/v4/", "openai_api": "chat_completions" },
            "context": { "openai": 64000 },
        }))],
    );
    run_step(&w);
    let t = accounts(&w);
    let a = &t["accounts"][0];
    assert_eq!(a["kind"], "openai_compatible");
    assert_eq!(a["base_url"], "https://api.z.ai/api/paas/v4");
    assert_eq!(a["model"], "glm-5.3-flash");
    assert_eq!(a["context"], json!({ "window": 64000 }));
    // No key was stored: a keyless account at its own URL.
    let id = id_of(&t, "OpenAI");
    assert_eq!(
        secret(&w, &format!("ai_account_key_{}", id.to_ascii_lowercase())),
        None
    );
    // A URL with the Responses API stays an OpenAI API account at that URL.
    let dir = tempfile::tempdir().unwrap();
    let w = vault(
        dir.path(),
        &[settings(
            json!({ "endpoints": { "openai": "https://openrouter.ai/api/v1" } }),
        )],
    );
    run_step(&w);
    let a = &accounts(&w)["accounts"][0];
    assert_eq!(a["kind"], "openai");
    assert_eq!(a["base_url"], "https://openrouter.ai/api/v1");
}

#[test]
fn a_chatgpt_sign_in_moves_to_its_account() {
    let dir = tempfile::tempdir().unwrap();
    let auth = json!({
        "client_id": "oaiapp_1", "host_id": "urn:uuid:x", "access_token": "at",
        "refresh_token": "rt", "expires_ms": 1, "account": "me@example.com",
    });
    let w = vault(
        dir.path(),
        &[
            ("ai_chatgpt", auth.to_string()),
            settings(json!({ "models": { "chatgpt": "gpt-6.1-sol" } })),
        ],
    );
    run_step(&w);
    let t = accounts(&w);
    let a = &t["accounts"][0];
    assert_eq!(
        (a["name"].as_str(), a["kind"].as_str()),
        (Some("ChatGPT"), Some("chatgpt"))
    );
    assert_eq!(a["model"], "gpt-6.1-sol");
    let id = id_of(&t, "ChatGPT");
    let moved: Value = serde_json::from_str(
        &secret(
            &w,
            &format!("ai_account_chatgpt_{}", id.to_ascii_lowercase()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(moved, auth);
    assert_eq!(secret(&w, "ai_chatgpt"), None);
}

#[test]
fn all_three_keep_their_order_and_the_default_provider_is_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let w = vault(
        dir.path(),
        &[
            ("ai_anthropic_api_key", "sk-ant".into()),
            ("ai_openai_api_key", "sk-oa".into()),
            (
                "ai_chatgpt",
                json!({ "access_token": "at", "refresh_token": "rt" }).to_string(),
            ),
            settings(json!({
                "default_provider": "chatgpt",
                "models": { "anthropic": "claude-opus-5", "openai": "gpt-6" },
                "endpoints": { "anthropic": "http://10.0.0.5:11434", "chatgpt": "http://seam" },
                "system_prompt": "kept",
            })),
        ],
    );
    run_step(&w);
    let t = accounts(&w);
    let names: Vec<&str> = t["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Anthropic", "OpenAI", "ChatGPT"]);
    assert_eq!(t["accounts"][0]["base_url"], "http://10.0.0.5:11434");
    assert_eq!(t["default"], json!(id_of(&t, "ChatGPT")));
    assert_eq!(
        t["legacy"],
        json!({
            "anthropic": id_of(&t, "Anthropic"),
            "openai": id_of(&t, "OpenAI"),
            "chatgpt": id_of(&t, "ChatGPT"),
        })
    );
    // The rest of ai_settings stays, the test seam among it.
    let s: Value = serde_json::from_str(&secret(&w, "ai_settings").unwrap()).unwrap();
    assert_eq!(s["endpoints"]["chatgpt"], "http://seam");
    for old in ["ai_anthropic_api_key", "ai_openai_api_key", "ai_chatgpt"] {
        assert_eq!(secret(&w, old), None, "{old}");
    }
}

#[test]
fn the_step_is_idempotent_and_a_fresh_vault_gets_no_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let w = vault(dir.path(), &[("ai_anthropic_api_key", "sk-ant".into())]);
    run_step(&w);
    let first = accounts(&w);
    // Old secrets arriving again, as from a backup, change nothing once accounts exist.
    w.state().set_secret("ai_openai_api_key", "sk-oa").unwrap();
    let step = writer::UPGRADE_STEPS
        .iter()
        .find(|s| s.id == "0010_ai_accounts")
        .unwrap();
    w.run_upgrade_steps(&[UpgradeStep {
        id: "test_0010_ai_accounts_again",
        run: step.run,
    }])
    .unwrap();
    assert_eq!(accounts(&w), first);
    assert_eq!(secret(&w, "ai_openai_api_key").as_deref(), Some("sk-oa"));

    // A vault with nothing set up writes no table.
    let dir = tempfile::tempdir().unwrap();
    let w = vault(dir.path(), &[]);
    run_step(&w);
    assert_eq!(secret(&w, "ai_accounts"), None);
}

#[test]
fn without_the_secrets_key_the_step_defers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = vault(root, &[("ai_anthropic_api_key", "sk-ant".into())]);
    drop(w);
    let vault = Vault::open(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    let w = Writer::open(vault, state).unwrap();
    assert!(run_step(&w).is_empty(), "deferred, not recorded");
}

const LEGACY: &str = "label = \"Helper\"\nprovider = \"openai\"\nmodel = \"gpt-6\"\n";

#[test]
fn an_agent_naming_a_provider_names_its_account_on_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = vault(
        root,
        &[
            ("ai_openai_api_key", "sk-oa".into()),
            (
                "ai_chatgpt",
                json!({ "access_token": "a", "refresh_token": "r" }).to_string(),
            ),
        ],
    );
    run_step(&w);
    // Files of before accounts, arriving as a device or an older app writes them.
    w.put_agent("helper", LEGACY, |_| Vec::new()).unwrap();
    w.put_agent(
        "planner",
        "label = \"Planner\"\nprovider = \"chatgpt\"\n",
        |_| Vec::new(),
    )
    .unwrap();
    // No account was made for Anthropic: that file is left as it is.
    w.put_agent("old", "provider = \"anthropic\"\n", |_| Vec::new())
        .unwrap();
    let before = generation(root);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(generation(root), before + 1, "one commit");
    let helper = w.agent("helper").unwrap();
    assert_eq!(helper.account.as_deref(), Some("OpenAI"));
    assert_eq!(helper.provider, None);
    assert_eq!(helper.model.as_deref(), Some("gpt-6"));
    let text = w.read_agent("helper").unwrap();
    assert!(text.contains("account = \"OpenAI\""), "{text}");
    assert!(!text.contains("provider"), "{text}");
    assert_eq!(
        w.agent("planner").unwrap().account.as_deref(),
        Some("ChatGPT")
    );
    assert_eq!(
        w.agent("old").unwrap().provider.as_deref(),
        Some("anthropic")
    );

    // Idempotent: the next start commits nothing.
    let after = generation(root);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(generation(root), after);
}
