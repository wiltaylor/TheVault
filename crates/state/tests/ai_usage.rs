use state::State;
use tempfile::TempDir;

struct FixedClock(i64);
impl state::Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[test]
fn utc_days_retention_boundary_and_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let now = 200 * 86_400_000;
    let clock = std::sync::Arc::new(FixedClock(now));
    let s = State::open_with_clock(&path, None, clock.clone()).unwrap();
    for _ in 0..3 {
        s.record_ai_usage("a", "m", "agent", 2, 3).unwrap();
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("UPDATE ai_usage SET at_ms=?1 WHERE rowid=1", [now - 1])
        .unwrap();
    let rows = s.ai_usage_daily().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].day.as_str(), rows[0].calls), ("1970-07-20", 2));
    assert_eq!((rows[1].day.as_str(), rows[1].calls), ("1970-07-19", 1));
    conn.execute(
        "UPDATE ai_usage SET at_ms=?1 WHERE rowid=1",
        [now - state::AI_USAGE_RETENTION_MS],
    )
    .unwrap();
    conn.execute(
        "UPDATE ai_usage SET at_ms=?1 WHERE rowid=2",
        [now - state::AI_USAGE_RETENTION_MS - 1],
    )
    .unwrap();
    s.prune_activity().unwrap();
    drop(s);
    let reopened = State::open_with_clock(&path, None, clock).unwrap();
    assert_eq!(
        reopened
            .ai_usage_daily()
            .unwrap()
            .iter()
            .map(|r| r.calls)
            .sum::<i64>(),
        2
    );
}

#[test]
fn daily_usage_and_retention() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let s = State::open(&path, None).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    for (purpose, input, output) in [("classifier", 12, 3), ("classifier", 8, 2), ("agent", 9, 1)] {
        s.record_ai_usage("account", "model", purpose, input, output)
            .unwrap();
    }
    let rows = s.ai_usage_daily().unwrap();
    assert_eq!(rows.len(), 2);
    let classifier = rows.iter().find(|r| r.purpose == "classifier").unwrap();
    assert_eq!(
        (
            classifier.calls,
            classifier.input_tokens,
            classifier.output_tokens
        ),
        (2, 20, 5)
    );
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE ai_usage SET at_ms=?1 WHERE purpose='agent'",
        [now - state::AI_USAGE_RETENTION_MS - 1],
    )
    .unwrap();
    s.prune_activity().unwrap();
    assert_eq!(s.ai_usage_daily().unwrap().len(), 1);
}
