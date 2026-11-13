//! The built-in `agent_thread` type in new and existing vaults (agents G7), and step
//! `0006_assistant_settings`, which moves the chat settings into the built-in Assistant
//! (agents G6).

use std::path::Path;

use serde_json::{Value, json};
use state::{SecretKey, State};
use store::Vault;
use writer::{AGENT_THREAD_TYPE, UpgradeStep, Upkeep, Writer};

#[test]
fn browser_secrets_agent_script_reads_are_redacted_before_transformation() {
    use script_core::Vault as _;
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let transcript = json!({"messages":[],"proposals":[{"kind":"agent","name":"helper","before":"permissions {\n browser = true\n browser_secrets = [\"bank\"]\n}\n"}]}).to_string();
    let note = w.agent_thread_runtime().create(&writer::Input::Json(json!({"fields":{"title":"Thread","agent":"assistant","origin":"user","transcript":transcript}})), &state::Author::Agent("assistant".into())).unwrap();
    assert!(
        script_core::Vault::get(&w, &note.id)
            .unwrap()
            .to_string()
            .contains("bank")
    );
    {
        let _acting = writer::acting_as(&state::Author::Agent("reader".into()));
        let read = script_core::Vault::get(&w, &note.id).unwrap();
        assert!(!read.to_string().contains("bank"));
        for query in [
            "from agent_thread",
            "from agent_thread select transcript",
            "from agent_thread group by transcript",
        ] {
            let read = w.query_page(query, 100, None).unwrap();
            assert!(!read.to_string().contains("bank"), "{query}");
        }
    }
    assert!(w.read(&note.id).unwrap().wcl.contains("bank"));
}

#[test]
fn external_tokens_cannot_write_memory_even_without_api_guards() {
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let user = state::Author::Token("writer".into());
    let external = state::Author::Agent("agent-external".into());
    let input = writer::Input::Json(json!({"fields": {"title": "Rule", "agent": "assistant"}}));
    assert!(matches!(
        w.create("agent_memory", &input, &external),
        Err(writer::Error::AgentMemoryToken)
    ));
    let m = w.create("agent_memory", &input, &user).unwrap();
    assert!(matches!(
        w.update(&m.id, &input, Some(&m.version), &external),
        Err(writer::Error::AgentMemoryToken)
    ));
    assert!(matches!(
        w.set_archived(&m.id, true, Some(&m.version), &external),
        Err(writer::Error::AgentMemoryToken)
    ));
    assert!(matches!(
        w.delete(&m.id, Some(&m.version), &external),
        Err(writer::Error::AgentMemoryToken)
    ));
    assert!(matches!(
        w.restore_version(&m.id, &m.version, Some(&m.version), &external),
        Err(writer::Error::AgentMemoryToken)
    ));
    w.delete(&m.id, Some(&m.version), &user).unwrap();
    assert!(matches!(
        w.restore_from_trash(&m.id, &external),
        Err(writer::Error::AgentMemoryToken)
    ));
    assert!(matches!(
        w.create_remote(
            "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA",
            "agent_memory",
            "title = \"Injected\"\nagent = \"assistant\"\n",
            &external
        ),
        Err(writer::Error::AgentMemoryToken)
    ));
}

#[test]
fn gitsync_cannot_create_agent_transcripts() {
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let peer = state::Author::Task("gitsync peer".into());
    assert!(matches!(
        w.create_remote(
            "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA",
            "agent_thread",
            "title = \"Forged\"\n",
            &peer
        ),
        Err(writer::Error::ServerOwnedThread)
    ));
}

#[test]
fn gitsync_cannot_replace_agent_transcripts() {
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let peer = state::Author::Task("gitsync peer".into());
    let thread = w.agent_thread_runtime().create(&writer::Input::Json(json!({"fields": {"title": "Thread", "agent": "assistant", "origin": "user", "transcript": "{}"}})), &state::Author::Agent("assistant".into())).unwrap();
    assert!(matches!(
        w.replace_remote(
            &thread.id,
            "agent_thread",
            "title = \"Forged\"\n",
            &thread.version,
            &peer
        ),
        Err(writer::Error::ServerOwnedThread)
    ));
    assert!(matches!(
        w.replace_remote(
            &thread.id,
            "fleeting",
            "title = \"Retyped\"\n",
            &thread.version,
            &peer
        ),
        Err(writer::Error::ServerOwnedThread)
    ));
}

#[test]
fn gitsync_cannot_retype_a_note_into_an_agent_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let peer = state::Author::Task("gitsync peer".into());
    let note = w
        .create(
            "fleeting",
            &writer::Input::Json(json!({"fields": {"title": "Note"}})),
            &peer,
        )
        .unwrap();
    assert!(matches!(
        w.replace_remote(
            &note.id,
            "agent_thread",
            "title = \"Forged\"\n",
            &note.version,
            &peer
        ),
        Err(writer::Error::ServerOwnedThread)
    ));
}

#[test]
fn device_mirror_still_receives_server_transcripts() {
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let id = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
    w.apply_remote(&writer::Remote::Put {
        id: id.into(),
        type_name: "agent_thread".into(),
        wcl: "title = \"Mirror\"\nagent = \"assistant\"\norigin = \"user\"\ntranscript = \"{}\"\n"
            .into(),
    })
    .unwrap();
    assert_eq!(w.read(id).unwrap().type_name, "agent_thread");
}

#[test]
fn transcript_content_needs_runtime_capability_not_an_agent_author() {
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let input = writer::Input::Json(
        json!({"fields": {"title": "Thread", "agent": "assistant", "origin": "user", "transcript": "{\"messages\": []}"}}),
    );
    let author = state::Author::Agent("assistant".into());
    assert!(matches!(
        w.create("agent_thread", &input, &author),
        Err(writer::Error::ServerOwnedThread)
    ));
    let made = w.agent_thread_runtime().create(&input, &author).unwrap();
    for by in [
        author.clone(),
        state::Author::Session("owner".into()),
        state::Author::Token("admin".into()),
    ] {
        assert!(matches!(
            w.update(&made.id, &input, Some(&made.version), &by),
            Err(writer::Error::ServerOwnedThread)
        ));
        assert!(matches!(
            w.restore_version(&made.id, &made.version, Some(&made.version), &by),
            Err(writer::Error::ServerOwnedThread)
        ));
        assert!(matches!(
            w.put_canvas(&made.id, "body.0", "canvas {}", Some(&made.version), &by),
            Err(writer::Error::ServerOwnedThread)
        ));
    }
    let updated = w
        .agent_thread_runtime()
        .update(&made.id, &input, Some(&made.version), &author)
        .unwrap();
    let archived = w
        .set_archived(&made.id, true, Some(&updated.version), &author)
        .unwrap();
    w.delete(&made.id, Some(&archived.version), &author)
        .unwrap();
    assert!(matches!(
        w.restore_from_trash(&made.id, &author),
        Err(writer::Error::ServerOwnedThread)
    ));
    w.restore_from_trash(&made.id, &state::Author::Session("owner".into()))
        .unwrap();
}

#[test]
fn gmail_metadata_is_owner_or_import_only() {
    use state::Author;
    use writer::Input;
    let tmp = tempfile::tempdir().unwrap();
    let w = open(tmp.path(), None);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    for field in ["gmail_ref", "gmail_account", "gmail_deferred_parts"] {
        let value = if field == "gmail_deferred_parts" { json!(["part"]) } else { json!("value") };
        let input = Input::Json(json!({"fields":{"title":"Email",field:value}}));
        for author in [Author::Agent("agent-token".into()), Author::AgentMail("assistant".into()), Author::Handler("script".into()), Author::Task("other".into()), Author::Token("user".into())] {
            assert!(w.create("email", &input, &author).is_err(), "{field} {author:?}");
            let empty = Input::Json(json!({"fields":{"title":"Email"}}));
            let note = w.create("email", &empty, &Author::Session("owner".into())).unwrap();
            assert!(w.update(&note.id, &input, Some(&note.version), &author).is_err(), "{field} {author:?}");
        }
        for author in [Author::Session("owner".into()), Author::Task("google_gmail".into())] {
            let note = w.create("email", &input, &author).unwrap();
            let empty = Input::Json(json!({"fields":{"title":"Email"}}));
            w.update(&note.id, &empty, Some(&note.version), &author).unwrap();
        }
    }
}

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
