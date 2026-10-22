//! A scheduled task calling `ai::complete` against the fake Anthropic, its trail line kept with
//! the run in `task_runs` (ai D12, D13).

use std::sync::Arc;

use ai::{Endpoints, Models, Provider, Settings};
use scheduler::Scheduler;
use script::{Ai, Host, Limits};
use state::{RunOutcome, SecretKey, State, SystemClock};
use testkit::{AnthropicMessage, FakeAnthropic};
use writer::Writer;

const SUMMARISE: &str = r#"use ai
use vault

fn run() -> Result[string, string] {
    match ai::complete("Summarise the private roof notes", #{ "max_tokens": "100" }) {
        Ok(r) => {
            vault::log("summary: " + r.text)
            Ok(r.text)
        },
        Err(e) => Err(e.kind + ": " + e.message),
    }
}
"#;

#[test]
fn a_scheduled_task_calls_ai_and_its_run_keeps_the_trail_line() {
    let dir = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    let state = State::open(
        &vault.server_dir().join("state.db"),
        Some(SecretKey::generate()),
    )
    .unwrap();
    let writer = Arc::new(Writer::open(vault, state).unwrap());
    let fake = FakeAnthropic::start();
    Settings {
        default_provider: Some(Provider::Anthropic),
        models: Models {
            anthropic: Some("claude-test".into()),
            ..Models::default()
        },
        endpoints: Endpoints {
            anthropic: Some(fake.base_url()),
            ..Endpoints::default()
        },
        ..Settings::default()
    }
    .save(writer.state())
    .unwrap();
    writer
        .state()
        .set_secret(ai::ANTHROPIC_API_KEY, fake.api_key())
        .unwrap();
    fake.push(AnthropicMessage::text("Tiles are due.").usage(30, 4));

    let host = Arc::new(Host::new(
        writer.clone(),
        1,
        Limits::default(),
        Ai::of(&writer),
    ));
    let scheduler = Scheduler::new(writer.clone(), host, Arc::new(SystemClock)).unwrap();
    writer.put_script("summarise", SUMMARISE).unwrap();
    // The task names no secret and no host: `ai` reads its credentials itself.
    scheduler
        .put_task(
            "summarise",
            "schedule = \"0 0 1 1 *\"\nscript = \"summarise\"\n",
        )
        .unwrap();

    let run = scheduler.trigger("summarise").unwrap();
    assert_eq!(run.outcome, RunOutcome::Succeeded, "{}", run.log);
    let kept = writer.state().runs("summarise", 10).unwrap();
    assert_eq!(kept.len(), 1);
    let lines: Vec<&str> = kept[0].log.lines().collect();
    assert_eq!(lines.len(), 2, "{}", kept[0].log);
    assert!(
        lines[0].starts_with("ai: provider=anthropic model=claude-test in=30 out=4 outcome=ok ms="),
        "{}",
        lines[0]
    );
    assert_eq!(lines[1], "summary: Tiles are due.");
    assert!(!kept[0].log.contains("roof"), "{}", kept[0].log);
    assert_eq!(fake.message_requests()[0].json()["max_tokens"], 100);
}
