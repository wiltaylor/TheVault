//! The built-in `agent_thread` type in new and existing vaults (agents G7), and step
//! `0006_assistant_settings`, which moves the chat settings into the built-in Assistant
//! (agents G6).

use std::path::Path;

use serde_json::{Value, json};
use state::{SecretKey, State};
use store::Vault;
use writer::{AGENT_THREAD_TYPE, UpgradeStep, Upkeep, Writer};

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

fn settings(w: &Writer) -> Value {
    serde_json::from_str(&w.state().get_secret("ai_settings").unwrap().unwrap()).unwrap()
}

/// Run step `0006_assistant_settings` again under a new id, as a vault that has not run it yet
/// would on its next start.
fn run_step(w: &Writer) -> Vec<String> {
    let step = writer::UPGRADE_STEPS
        .iter()
        .find(|s| s.id == "0006_assistant_settings")
        .unwrap();
    w.run_upgrade_steps(&[UpgradeStep {
        id: "test_0006_assistant_settings",
        run: step.run,
    }])
    .unwrap()
}

#[test]
fn a_new_vault_gets_the_agent_thread_type_in_its_one_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root, Some(SecretKey::generate()));
    let before = generation(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(generation(root), before + 1);
    assert!(done.types_added.iter().any(|t| t == AGENT_THREAD_TYPE));
    assert!(done.steps.iter().any(|s| s == "0006_assistant_settings"));
    let schema = w.schema();
    let ty = schema.type_def(AGENT_THREAD_TYPE).unwrap();
    for f in [
        "agent",
        "origin",
        "last_activity",
        "pending",
        "transcript",
        "body",
    ] {
        assert!(ty.field(f).is_some(), "{f}");
    }
    assert!(ty.field("transcript").unwrap().hidden);
    assert!(w.is_built_in(AGENT_THREAD_TYPE));
    let page = ty.list_page.as_ref().unwrap();
    assert_eq!(page.sort, Some(("last_activity".to_owned(), true)));
    assert_eq!(page.view.as_deref(), Some("details"));
}

#[test]
fn an_existing_vault_gets_the_agent_thread_type_on_its_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root, None);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    // A vault from before agent threads.
    std::fs::remove_file(root.join("types/agent_thread.wcl")).unwrap();
    let w = open(root, None);
    assert!(w.schema().type_def(AGENT_THREAD_TYPE).is_none());
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(done.types_added, [AGENT_THREAD_TYPE]);
    assert!(w.schema().type_def(AGENT_THREAD_TYPE).is_some());
}

#[test]
fn the_chat_settings_move_into_the_assistant_and_leave_ai_settings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root, Some(SecretKey::generate()));
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    w.state()
        .set_secret(
            "ai_settings",
            &json!({
                "default_provider": "anthropic",
                "models": { "anthropic": "claude-opus-5" },
                "system_prompt": "Answer in French.",
                "confirm": { "notes": false, "types": true },
                "web": { "read": false },
            })
            .to_string(),
        )
        .unwrap();
    let before = generation(root);

    assert_eq!(run_step(&w), ["test_0006_assistant_settings"]);

    assert_eq!(generation(root), before + 1);
    let a = w.agent("assistant").unwrap();
    assert_eq!(a.system_prompt.as_deref(), Some("Answer in French."));
    assert!(!a.permissions.confirm.notes);
    assert!(a.permissions.confirm.types && a.permissions.confirm.delete);
    assert!(!a.permissions.web.read);
    assert!(a.permissions.web.search);
    assert_eq!(
        settings(&w),
        json!({ "default_provider": "anthropic", "models": { "anthropic": "claude-opus-5" } })
    );
    // The edited Assistant is kept on the next start, with no notice of a newer version.
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.files_upgraded.is_empty() && done.files_kept.is_empty());
    assert_eq!(w.agent("assistant").unwrap(), a);
}

#[test]
fn the_assistants_own_prompt_and_switches_win_and_defaults_change_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root, Some(SecretKey::generate()));
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    w.put_agent(
        "assistant",
        "label = \"Assistant\"\nsystem_prompt = \"Mine.\"\n\npermissions {\n  confirm {\n    delete = false\n  }\n}\n",
        |_| Vec::new(),
    )
    .unwrap();
    let mine = w.agent("assistant").unwrap();
    w.state()
        .set_secret(
            "ai_settings",
            &json!({ "system_prompt": "Theirs.", "confirm": { "notes": false } }).to_string(),
        )
        .unwrap();
    let before = generation(root);
    run_step(&w);
    assert_eq!(
        generation(root),
        before,
        "nothing was moved, so nothing was written"
    );
    assert_eq!(w.agent("assistant").unwrap(), mine);
    assert_eq!(settings(&w), json!({}));
}

#[test]
fn a_vault_from_before_agents_gets_an_assistant_holding_its_settings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let key = SecretKey::generate();
    {
        let w = open(root, Some(key.clone()));
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    std::fs::remove_file(root.join("agents/assistant.wcl")).unwrap();
    std::fs::remove_dir(root.join("agents")).unwrap();
    let w = open(root, Some(key));
    w.state()
        .set_secret(
            "ai_settings",
            &json!({ "confirm": { "archive": false } }).to_string(),
        )
        .unwrap();
    run_step(&w);
    let a = w.agent("assistant").unwrap();
    assert!(a.is_built_in());
    assert_eq!(a.label, "Assistant");
    assert!(!a.permissions.confirm.archive);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.files_added.is_empty());
    assert!(done.files_kept.is_empty(), "no notice of a newer version");
    assert_eq!(w.agent("assistant").unwrap(), a);
}
