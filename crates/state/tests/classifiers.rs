use state::State;
use tempfile::TempDir;

#[test]
fn incoming_asset_owners_are_sticky_atomic_and_unknown_marks_stay_unknown() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = std::sync::Arc::new(State::open(&path, None).unwrap());
    let threads: Vec<_> = (0..8).map(|i| {
        let state = state.clone();
        std::thread::spawn(move || {
            state.mark_incoming_asset("asset", None).unwrap();
            state.mark_incoming_asset("asset", Some(&format!("owner-{i}"))).unwrap();
        })
    }).collect();
    for thread in threads { thread.join().unwrap(); }
    let raw = state.meta("agent_safety.asset_owners.asset").unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Vec<String>>(&raw).unwrap().len(), 8);
    state.mark_incoming_asset("asset", None).unwrap();
    assert_eq!(state.meta("agent_safety.asset_owners.asset").unwrap().unwrap(), raw);
    state.set_meta("agent_safety.asset.old", "1").unwrap();
    state.mark_incoming_asset("old", Some("new-owner")).unwrap();
    assert!(state.meta("agent_safety.asset_owners.old").unwrap().is_none());
    drop(state);
    assert_eq!(State::open(&path, None).unwrap().meta("agent_safety.asset_owners.asset").unwrap().unwrap(), raw);
}

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
