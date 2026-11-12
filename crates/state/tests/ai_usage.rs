use state::State;
use tempfile::TempDir;

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
