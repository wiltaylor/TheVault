//! Exercise the actual scheduler -> TriggerRunner -> unattended Turn -> subagent
//! path. Writer hooks observe the contexts inside the Turn's blocking workers.
use super::*;
use scheduler::{AgentRunner, TurnReport};
use std::sync::{Mutex, mpsc as sync};
use std::time::Duration;
use testkit::{AiSeed, AnthropicMessage, AnthropicReply, FakeAnthropic, TestAccount};

struct Probe(Arc<Mutex<Vec<(Author, RunContext)>>>);

#[test]
fn run_unattended_marked_thread_without_incoming_source_keeps_action_guards() {
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
    let w = Arc::new(Writer::open(vault, state).unwrap());
    w.ensure_built_in_types().unwrap();
    let fake = FakeAnthropic::start();
    AiSeed::new()
        .account(TestAccount::anthropic(&fake).model("claude-test"))
        .store(w.state());
    w.put_agent("keeper", "label = \"Keeper\"\npermissions {\n scripts { run_code = true }\n confirm { delete = false }\n outbox { send_without_review = true }\n}\n", |_| Vec::new()).unwrap();
    let author = Author::Session("owner".into());
    let victim = w
        .create(
            "item",
            &writer::Input::Json(json!({"fields":{"title":"Keep"}})),
            &author,
        )
        .unwrap();
    let draft = w.create("outbox", &writer::Input::Json(json!({"fields":{"title":"Held", "kind":"email", "status":"draft", "to":["nobody@example.test"], "subject":"Held"}})), &author).unwrap();
    let web = testkit::StandIn::start();
    web.route("/send", testkit::Reply::text(200, "unexpected"));
    let agent = w.agent("keeper").unwrap();
    let host = Arc::new(script::Host::new(
        w.clone(),
        1,
        script::Limits::default(),
        script::Ai::of(&w),
    ));
    let scheduler =
        scheduler::Scheduler::new(w.clone(), host, Arc::new(state::SystemClock)).unwrap();
    crate::script_tools::register(&w, &scheduler);
    let thread = crate::agent_threads::create_with(
        &w,
        "keeper",
        Some("Marked"),
        scheduler::DISPATCH_ORIGIN,
        &[],
    )
    .unwrap();
    w.state()
        .set_meta(&format!("outside_text.thread.{}", thread.id), "1")
        .unwrap();
    fake.push(AnthropicMessage::new()
        .tool_use("draft", "draft_email", json!({"to":["nobody@example.test"],"cc":[],"bcc":[],"attachments":[],"in_reply_to":"","subject":"Review me","body":"Draft only"}))
        .tool_use("proposal", "propose_delete", json!({"id":victim.id}))
        .tool_use("delete", "run_script", json!({"code":format!("use vault\nfn main() -> Result[Value, string] {{ vault::delete(\"{}\", \"{}\") }}\n", victim.id, victim.version), "function":null, "args":null}))
        .tool_use("send", "run_script", json!({"code":format!("use vault\nfn main() -> Result[Value, string] {{ vault::update(\"{}\", Value::Map(#{{\"fields\": Value::Map(#{{\"status\": Value::String(\"sending\")}})}}), \"{}\") }}\n", draft.id, draft.version), "function":null, "args":null}))
        .tool_use("http", "run_script", json!({"code":format!("use http\nfn main() -> Result[int, string] {{ Ok(http::get(\"{}\", #{{}})?.status) }}\n", web.url("/send")), "function":null, "args":null}))
        .tool_use("task", "run_task", json!({"name":"google_gmail"})));
    fake.push(AnthropicMessage::text("Review required."));
    // No incoming note or inherited restriction: only the persisted thread marker grants it.
    let context = RunContext::default();
    assert!(!context.is_restricted());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let report = runtime.block_on(run_unattended(
        w.clone(),
        &agent,
        &thread.id,
        "Continue",
        crate::ai_retry::Waits::instant(),
        None,
        None,
        context,
    ));
    assert!(matches!(report.ended, Some(Ok(()))), "{:?}", report.ended);
    let requests = fake.message_requests();
    assert_eq!(requests.len(), 2);
    let results = requests[1].json()["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["content"]
        .to_string();
    assert!(results.contains("draft"), "{results}");
    assert!(
        results.matches(script::OUTSIDE_TEXT_REFUSAL).count() >= 4,
        "{results}"
    );
    assert_eq!(w.read(&victim.id).unwrap().version, victim.version);
    assert_eq!(w.read(&draft.id).unwrap().version, draft.version);
    assert!(web.requests().is_empty());
    let saved = crate::agent_threads::Thread::read(&w, &thread.id).unwrap();
    assert_eq!(saved.transcript.pending(), 1);
}

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
