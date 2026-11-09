use state::State;
use tempfile::TempDir;

#[test]
fn pending_creates_survive_reopen_and_drain_once_in_order_beyond_256() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let payloads: Vec<_> = (0..300).map(|i| format!("snapshot-{i}")).collect();
    state.queue_classifications(&payloads).unwrap();
    drop(state);
    let state = State::open(&path, None).unwrap();
    for payload in payloads {
        assert_eq!(state.take_classification().unwrap(), Some(payload));
    }
    assert_eq!(state.take_classification().unwrap(), None);
    drop(state);
    assert_eq!(
        State::open(&path, None)
            .unwrap()
            .take_classification()
            .unwrap(),
        None
    );
}
