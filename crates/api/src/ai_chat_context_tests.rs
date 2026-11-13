//! Exercise the actual scheduler -> TriggerRunner -> unattended Turn -> subagent
//! path. Writer hooks observe the contexts inside the Turn's blocking workers.
use super::*;
use scheduler::{AgentRunner, TurnReport};
use std::sync::{Mutex, mpsc as sync};
use std::time::Duration;
use testkit::{AiSeed, AnthropicMessage, AnthropicReply, FakeAnthropic, TestAccount};

struct Probe(Arc<Mutex<Vec<(Author, RunContext)>>>);

impl writer::Hooks for Probe {
    fn after(&self, saved: &[writer::Saved], author: &Author, _: &schema::Schema) {
        for change in saved {
            if change.type_name == "item" {
                self.0.lock().unwrap().push((
                    author.clone(),
                    RunContext::current().expect("Turn blocking worker context"),
                ));
            }
        }
    }
}

struct Runner {
    real: crate::agent_runs::TriggerRunner,
    context: RunContext,
    finished: sync::Sender<TurnReport>,
}

impl AgentRunner for Runner {
    fn open_thread(
        &self,
        agent: &Agent,
        origin: &str,
        title: &str,
        notes: &[String],
    ) -> Result<String, String> {
        self.real.open_thread(agent, origin, title, notes)
    }

    fn run_turn(
        &self,
        agent: &Agent,
        thread: &str,
        prompt: &str,
        scope: Option<Scope>,
        caller: Option<&Author>,
    ) -> TurnReport {
        assert_eq!(RunContext::current(), Some(self.context.clone()));
        let report = self.real.run_turn(agent, thread, prompt, scope, caller);
        self.finished.send(report.clone()).unwrap();
        report
    }
}

#[test]
fn restricted_agent_and_subagent_turns_share_context_without_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let vault = store::Vault::init(dir.path()).unwrap();
    std::fs::write(
        dir.path().join("types/item.wcl"),
        "label = \"Item\"\ntemplate default { tab \"Main\" { slot \"title\" } }\n",
    )
    .unwrap();
    let state = state::State::open(
        &vault.server_dir().join("state.db"),
        Some(state::SecretKey::generate()),
    )
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let w = Arc::new(
        Writer::open(vault, state)
            .unwrap()
            .with_hooks(Box::new(Probe(seen.clone()))),
    );
    w.ensure_built_in_types().unwrap();
    let fake = FakeAnthropic::start();
    AiSeed::new()
        .account(TestAccount::anthropic(&fake).model("claude-test"))
        .store(w.state());
    w.put_agent(
        "lead",
        r#"label = "Lead"
permissions {
  tools = ["propose_create"]
  confirm { notes = false }
  subagents { allow = true }
}
"#,
        |_| Vec::new(),
    )
    .unwrap();
    let create = |id: &str, title: &str| {
        AnthropicMessage::new().tool_use(
            id,
            "propose_create",
            json!({"type": "item", "note": json!({"fields": {"title": title}}).to_string()}),
        )
    };
    let overloaded = || {
        AnthropicReply::error(529, "overloaded_error", "Overloaded").header("retry-after", "3600")
    };
    fake.push(overloaded());
    fake.push(create("parent", "Parent note"));
    fake.push(AnthropicMessage::new().tool_use("child", "start_subagent", json!({
        "task": "Make a note", "instructions": null, "agent": null, "permissions": null, "model": null
    })));
    fake.push(overloaded());
    fake.push(create("sub", "Child note"));
    fake.push(AnthropicMessage::text("Child done."));
    fake.push(AnthropicMessage::text("Parent done."));
    let context = RunContext::default();
    context.restrict();
    let generation = w.state().admit_automation().unwrap();
    context.capture_stop_generation(generation);
    let (finished, result) = sync::channel();
    let host = Arc::new(script::Host::new(
        w.clone(),
        1,
        script::Limits::default(),
        script::Ai::of(&w),
    ));
    let scheduler =
        scheduler::Scheduler::new(w.clone(), host, Arc::new(state::SystemClock)).unwrap();
    scheduler.set_agent_runner(Arc::new(Runner {
        real: crate::agent_runs::TriggerRunner::new(w, crate::ai_retry::Waits::instant()),
        context: context.clone(),
        finished,
    }));
    context
        .scope(|| {
            scheduler.dispatch(
                "lead",
                &[],
                "Provenance",
                "Make parent and child notes",
                Scope::Admin,
                Author::Session("owner".into()),
            )
        })
        .unwrap();
    let report = result.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(report.outcome.is_ok(), "{report:?}");
    let observations = seen.lock().unwrap();
    // Observe after-hooks, not previews: both turns committed a real note.
    assert_eq!(observations.len(), 2, "{observations:?}");
    for author in ["lead", "lead/sub:1"] {
        assert!(
            observations
                .iter()
                .any(|(by, _)| by == &Author::Agent(author.into())),
            "missing {author}: {observations:?}"
        );
    }
    for (_, observed) in observations.iter() {
        assert_eq!(observed, &context);
        assert!(observed.is_restricted());
        assert_eq!(observed.stop_generation(), Some(generation));
    }
    assert!(RunContext::current().is_none());
}
