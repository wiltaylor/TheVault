//! Rounds the scheduler starts on the cron schedule (git-sync D20), on a server whose clock
//! only the test moves.

use serde_json::{Value, json};
use testkit::{BareRepo, TestServer};

use crate::instance::{Instance, memo, seed};

fn move_clock(i: &Instance, by: &str) {
    i.call(
        "POST",
        "/api/test/clock",
        Some(json!({ "advance": by })),
        200,
    );
}

#[test]
fn the_schedule_starts_a_round_when_due_and_never_while_paused() {
    let repo = BareRepo::new();
    let server = TestServer::start_with_args(
        seed,
        &["--test-clock".as_ref(), "2026-01-01T00:00:00Z".as_ref()],
    );
    let a = Instance::on("A", server);
    a.generate_key();
    a.configure(
        &repo.url(),
        "from memo",
        "",
        json!({ "paused": false, "schedule": "*/15 * * * *" }),
    );
    a.create(&memo("Scheduled", "1", "1"));

    // The first tick learns the schedule; the round runs once it is due.
    move_clock(&a, "PT1M");
    assert_eq!(a.status()["last_run"], Value::Null);
    move_clock(&a, "PT10M");
    assert_eq!(a.status()["last_run"], Value::Null);
    move_clock(&a, "PT5M");
    let status = a.status();
    assert_eq!(status["last_run"], "2026-01-01T00:16:00Z", "{status}");
    assert_eq!(status["last_report"]["pushed"], true, "{status}");
    assert_eq!(repo.branches().len(), 1);

    // Paused, nothing runs however far the clock moves.
    a.configure(&repo.url(), "from memo", "", json!({ "paused": true }));
    move_clock(&a, "PT1H");
    assert_eq!(a.status()["last_run"], "2026-01-01T00:16:00Z");
}
