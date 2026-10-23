//! Agent files through the writer (agents G1, G2, G6): create, update and delete, the checks a
//! write runs, the built-in Assistant, and its arrival in a new and in an existing vault.

use std::path::Path;

use schema::{Agent, Issue};
use state::State;
use store::Vault;
use writer::{Error, Upkeep, Writer};

const EMAIL: &str = r#"label = "Email"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const TRIAGE: &str = r#"label = "Triage"

permissions {
  types = ["email"]
}

trigger new_email {
  on     = ["create"]
  type   = "email"
  prompt = "File this email."
}
"#;

fn open(root: &Path) -> Writer {
    let vault = Vault::init(root).unwrap();
    let state = State::open(&vault.server_dir().join("state.db"), None).unwrap();
    Writer::open(vault, state).unwrap()
}

fn with_email(root: &Path) -> Writer {
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/email.wcl"), EMAIL).unwrap();
    open(root)
}

fn none(_: &Agent) -> Vec<Issue> {
    Vec::new()
}

fn issues_of(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(inv) => inv.issues.iter().map(|i| i.to_string()).collect(),
        other => panic!("expected invalid, got {other:?}"),
    }
}

/// The synced files the change feed names, with whether each exists.
fn feed(w: &Writer) -> Vec<(String, bool)> {
    w.state()
        .changes_since(0, 10_000)
        .unwrap()
        .changes
        .into_iter()
        .filter_map(|c| match c.key {
            state::ChangeKey::File(path) => Some((path, c.present)),
            _ => None,
        })
        .collect()
}

#[test]
fn an_agent_is_created_updated_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = with_email(root);
    assert!(w.agents().unwrap().is_empty());

    let (text, agent) = w.put_agent("triage", TRIAGE, none).unwrap();
    assert_eq!(agent.label, "Triage");
    assert_eq!(
        std::fs::read_to_string(root.join("agents/triage.wcl")).unwrap(),
        text
    );
    assert_eq!(w.read_agent("triage").unwrap(), text);
    assert_eq!(w.agent("triage").unwrap(), agent);
    let listed = w.agents().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, "triage");
    assert_eq!(listed[0].1.as_ref().unwrap(), &agent);
    assert!(feed(&w).contains(&("agents/triage.wcl".to_owned(), true)));

    // Written in canonical form, whatever the spacing sent.
    let messy = "label=\"Triage 2\"\nmodel   =   \"gpt-6\"\n";
    let (text, agent) = w.put_agent("triage", messy, none).unwrap();
    assert_eq!(text, store::canonical(messy, "agents/triage.wcl").unwrap());
    assert_eq!(agent.label, "Triage 2");
    assert_eq!(agent.model.as_deref(), Some("gpt-6"));
    assert!(agent.triggers.is_empty());

    w.delete_agent("triage").unwrap();
    assert!(!root.join("agents/triage.wcl").exists());
    assert!(matches!(w.read_agent("triage"), Err(Error::Missing(_))));
    assert!(w.agents().unwrap().is_empty());
    assert!(feed(&w).contains(&("agents/triage.wcl".to_owned(), false)));
    assert!(matches!(w.delete_agent("triage"), Err(Error::Missing(_))));
}

#[test]
fn a_write_is_refused_with_every_problem_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = with_email(root);
    assert!(matches!(
        w.put_agent("Triage", TRIAGE, none),
        Err(Error::BadRequest(_))
    ));
    let bad = r#"provider = "gemini"

permissions {
  types = ["invoice"]
}

trigger t {
  schedule = "every day"
  prompt   = "Go."
}
"#;
    assert_eq!(
        issues_of(w.put_agent("triage", bad, none).unwrap_err()),
        [
            "provider: \"gemini\" is not one of anthropic, openai, chatgpt",
            "trigger t.schedule: \"every day\" is not a five-field cron expression: minute, hour, day of month, month, day of week",
        ]
    );
    // The file alone checks, so the types are checked next.
    let unknown = "permissions {\n  types = [\"invoice\"]\n}\n";
    assert_eq!(
        issues_of(w.put_agent("triage", unknown, none).unwrap_err()),
        ["permissions.types: there is no type \"invoice\""]
    );
    // And then what the caller knows, such as the tools.
    let tools = "permissions {\n  tools = [\"teleport\"]\n}\n";
    let check = |a: &Agent| {
        a.permissions
            .tools
            .iter()
            .flatten()
            .filter(|t| *t != "get_note")
            .map(|t| Issue::new("permissions.tools", format!("there is no tool {t:?}")))
            .collect()
    };
    assert_eq!(
        issues_of(w.put_agent("triage", tools, check).unwrap_err()),
        ["permissions.tools: there is no tool \"teleport\""]
    );
    assert!(matches!(
        w.put_agent("triage", "label = \"x\" + \"y\"\n", none),
        Err(Error::DataOnly(_) | Error::Parse { .. })
    ));
    assert!(!root.join("agents/triage.wcl").exists());
    // `check_agent` runs the same checks and writes nothing.
    assert!(w.check_agent("triage", unknown, none).is_err());
    let (text, _) = w.check_agent("triage", TRIAGE, none).unwrap();
    assert_eq!(text, store::canonical(TRIAGE, "agents/triage.wcl").unwrap());
    assert!(!root.join("agents/triage.wcl").exists());
}

#[test]
fn a_broken_file_is_listed_with_its_problems() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(root.join("agents/broken.wcl"), "effort = \"huge\"\n").unwrap();
    let w = open(root);
    let listed = w.agents().unwrap();
    assert_eq!(listed[0].0, "broken");
    assert_eq!(listed[0].1.as_ref().unwrap_err().issues[0].at, "effort");
    assert!(matches!(w.agent("broken"), Err(Error::Invalid(_))));
}

#[test]
fn a_new_vault_gets_the_built_in_assistant() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.files_added.iter().any(|f| f == "agents/assistant.wcl"));
    let a = w.agent("assistant").unwrap();
    assert!(a.is_built_in());
    assert_eq!(a.label, "Assistant");
    assert!(a.enabled);
    assert_eq!(a.provider, None);
    assert_eq!(a.model, None);
    assert_eq!(a.system_prompt, None);
    assert_eq!(a.permissions.tools, None);
    assert_eq!(a.permissions.types, None);
    assert!(a.triggers.is_empty());
    assert!(feed(&w).contains(&("agents/assistant.wcl".to_owned(), true)));
}

#[test]
fn an_existing_vault_gets_the_assistant_on_its_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let w = open(root);
        w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    }
    // A vault from before agents: no `agents/` folder, and every other shipped file in place.
    std::fs::remove_file(root.join("agents/assistant.wcl")).unwrap();
    std::fs::remove_dir(root.join("agents")).unwrap();
    let w = open(root);
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert_eq!(done.files_added, ["agents/assistant.wcl"]);
    assert!(done.types_added.is_empty());
    assert!(w.agent("assistant").unwrap().is_built_in());
    // A device's local vault gets it too, as it gets the other shipped files.
    let dir = tempfile::tempdir().unwrap();
    let w = open(dir.path());
    let done = w.upgrade_shipped_defaults(Upkeep::Device).unwrap();
    assert!(done.files_added.iter().any(|f| f == "agents/assistant.wcl"));
}

#[test]
fn the_assistant_cannot_be_deleted_or_disabled_but_can_change() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = open(root);
    w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    let before = w.read_agent("assistant").unwrap();

    match w.delete_agent("assistant") {
        Err(Error::BuiltIn(m)) => assert_eq!(m, "Assistant is built in and cannot be deleted."),
        other => panic!("{other:?}"),
    }
    match w.put_agent(
        "assistant",
        "label = \"Assistant\"\nenabled = false\n",
        none,
    ) {
        Err(Error::BuiltIn(m)) => assert_eq!(m, "Assistant is built in and cannot be disabled."),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.read_agent("assistant").unwrap(), before);

    let changed =
        "label = \"Assistant\"\nmodel = \"claude-sonnet-5\"\nsystem_prompt = \"Be brief.\"\n";
    let (_, a) = w.put_agent("assistant", changed, none).unwrap();
    assert_eq!(a.model.as_deref(), Some("claude-sonnet-5"));
    assert!(a.enabled);
    // An edited Assistant is the user's: the next start keeps it.
    let done = w.upgrade_shipped_defaults(Upkeep::Home).unwrap();
    assert!(done.files_upgraded.is_empty() && done.files_added.is_empty());
    assert_eq!(w.agent("assistant").unwrap(), a);
}
