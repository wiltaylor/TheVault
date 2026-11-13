//! Capacity retries of an unattended agent run's model calls (agents G12).
//!
//! When a model call of a trigger's run, a mention's, or one of their sub agents' fails with a
//! transient error ([`ai::AiError::is_transient`]: the provider is overloaded, at capacity or
//! unreachable, or a rate limit that is not a used-up quota), the turn waits [`CAPACITY_WAITS`]
//! in turn and makes the same call again. The fourth failure ends the run as any failure does.
//! Each wait is added to the run's log as it starts, so the agent's Triggers tab shows it while
//! the run waits. Interactive chat never retries this way.
//!
//! Only the failed call is made again, with the request it was made with: the tool calls of
//! earlier steps are already answered in that request, and a step's own tool calls run only once
//! its call has ended, so a failed call has run none. The text a failed call streamed is dropped.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use writer::Writer;

/// How long an unattended run waits before each retry of a model call that failed with a
/// transient error: three retries, so four attempts in all.
pub const CAPACITY_WAITS: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(240),
];

/// Waits for a duration.
pub type Sleep = Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// The waits before each retry and how they are slept.
#[derive(Clone)]
pub struct Waits {
    pub waits: Vec<Duration>,
    pub sleep: Sleep,
}

impl Waits {
    /// [`CAPACITY_WAITS`], slept on the async runtime's timer.
    pub fn real() -> Waits {
        Waits {
            waits: CAPACITY_WAITS.to_vec(),
            sleep: Arc::new(|d| Box::pin(tokio::time::sleep(d))),
        }
    }

    /// [`CAPACITY_WAITS`], each over at once: for a server run with `--test-clock`, whose tests
    /// read the waits from the run's log.
    pub fn instant() -> Waits {
        Waits {
            waits: CAPACITY_WAITS.to_vec(),
            sleep: Arc::new(|_| Box::pin(std::future::ready(()))),
        }
    }
}

/// The retries an unattended turn's model calls make, and where each wait is told: the run's
/// trail lines, and its record in `agent_runs` while it runs.
#[derive(Clone)]
pub(crate) struct CapacityRetry {
    pub(crate) waits: Waits,
    run_context: script::RunContext,
    writer: Arc<Writer>,
    /// The run's thread, which names its record.
    thread: String,
    report: crate::ai_chat::Report,
    /// For a sub agent's turn, its number.
    sub: Option<usize>,
}

impl CapacityRetry {
    pub(crate) fn new(
        waits: Waits,
        writer: Arc<Writer>,
        thread: &str,
        report: crate::ai_chat::Report,
        run_context: script::RunContext,
    ) -> CapacityRetry {
        CapacityRetry {
            waits,
            run_context: run_context.inherit_current(),
            writer,
            thread: thread.to_owned(),
            report,
            sub: None,
        }
    }

    /// The same retries for sub agent `n` of the run.
    pub(crate) fn for_sub(&self, n: usize) -> CapacityRetry {
        CapacityRetry {
            sub: Some(n),
            ..self.clone()
        }
    }

    /// The wait before retry `retry` (0 for the first), or `None` once none is left.
    pub(crate) fn wait(&self, retry: usize) -> Option<Duration> {
        self.waits.waits.get(retry).copied()
    }

    /// Add the line for the wait before retry `retry` after `error` to the run's log.
    pub(crate) async fn note(&self, retry: usize, wait: Duration, error: &ai::AiError) {
        let line = wait_line(
            self.sub,
            wait,
            retry + 2,
            self.waits.waits.len() + 1,
            &error.message,
        );
        eprintln!("agent run on thread {}: {line}", self.thread);
        self.report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .log
            .push(line.clone());
        let w = self.writer.clone();
        let thread = self.thread.clone();
        let _ = crate::ai_chat::blocking(&self.run_context, move || {
            if let Err(e) = w.state().append_agent_run_log(&thread, &line) {
                eprintln!("agent run on thread {thread}: recording a wait: {e}");
            }
        })
        .await;
    }

    pub(crate) async fn sleep(&self, wait: Duration) {
        (self.waits.sleep)(wait).await;
    }
}

/// "model at capacity; retrying in 60s (attempt 2 of 4): <error>".
fn wait_line(sub: Option<usize>, wait: Duration, attempt: usize, of: usize, error: &str) -> String {
    let who = sub.map_or_else(String::new, |n| format!("sub agent {n}: "));
    format!(
        "{who}model at capacity; retrying in {}s (attempt {attempt} of {of}): {}",
        wait.as_secs(),
        state::one_line(error, 200)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retry_and_subagent_keep_live_context_after_fake_wait() {
        let dir = tempfile::tempdir().unwrap();
        let vault = store::Vault::init(dir.path()).unwrap();
        let state = state::State::open(&vault.server_dir().join("state.db"), None).unwrap();
        let writer = Arc::new(Writer::open(vault, state).unwrap());
        let id = writer.state().start_agent_run("test", "manual").unwrap();
        writer.state().set_agent_run_thread(id, "thread").unwrap();
        let context = script::RunContext::default();
        let generation = writer.state().admit_automation().unwrap();
        context.capture_stop_generation(generation);
        let during_wait = context.clone();
        let waits = Waits {
            waits: CAPACITY_WAITS.to_vec(),
            sleep: Arc::new(move |_| {
                let context = during_wait.clone();
                Box::pin(async move {
                    assert!(script::RunContext::current().is_none());
                    tokio::task::yield_now().await;
                    context.restrict();
                })
            }),
        };
        let report = Arc::default();
        let retry = CapacityRetry::new(waits, writer.clone(), "thread", report, context.clone());
        let sub = retry.for_sub(1);
        retry.sleep(retry.wait(0).unwrap()).await;
        for item in [&retry, &sub] {
            assert_eq!(item.run_context, context);
            assert!(item.run_context.is_restricted());
            assert_eq!(item.run_context.stop_generation(), Some(generation));
            let observed = crate::ai_chat::blocking(&item.run_context, || {
                script::RunContext::current().unwrap()
            })
            .await
            .unwrap();
            assert_eq!(observed, context);
            item.note(
                0,
                item.wait(0).unwrap(),
                &ai::AiError::new(ai::ErrorKind::Unavailable, "fake capacity error"),
            )
            .await;
        }
        let log = writer.state().agent_run(id).unwrap().unwrap().log;
        assert_eq!(log.lines().count(), 2);
        assert!(log.contains("sub agent 1: model at capacity"));
        assert!(script::RunContext::current().is_none());
    }

    #[test]
    fn the_waits_are_one_two_and_four_minutes() {
        assert_eq!(
            CAPACITY_WAITS.map(|d| d.as_secs()),
            [60, 120, 240],
            "the schedule the docs state"
        );
    }

    #[test]
    fn a_wait_line_names_the_wait_and_the_attempt() {
        assert_eq!(
            wait_line(
                None,
                Duration::from_secs(60),
                2,
                4,
                "Anthropic m: Overloaded"
            ),
            "model at capacity; retrying in 60s (attempt 2 of 4): Anthropic m: Overloaded"
        );
        assert!(
            wait_line(Some(3), Duration::from_secs(240), 4, 4, "x")
                .starts_with("sub agent 3: model at capacity; retrying in 240s (attempt 4 of 4)")
        );
    }
}
