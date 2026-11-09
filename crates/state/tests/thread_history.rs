use state::{Author, State};
use tempfile::TempDir;

#[test]
fn saves_keep_three_and_trash_restore_keeps_content() {
    let dir = TempDir::new().unwrap();
    let state = State::open(&dir.path().join("state.db"), None).unwrap();
    let author = Author::Token("test".into());
    let mut tokens = Vec::new();
    for n in 0..10 {
        tokens.push(state.record_version("thread", "agent_thread", &author, &n.to_string()).unwrap().token);
        state.record_version("other", "email", &author, &n.to_string()).unwrap();
    }
    let history = state.history("thread").unwrap();
    assert_eq!(history.iter().map(|v| v.token.clone()).collect::<Vec<_>>(), tokens[7..]);
    assert_eq!(state.history("other").unwrap().len(), 10);
    assert!(state.version("thread", &tokens[0]).unwrap().is_none());
    assert_eq!(state.head("thread").unwrap().unwrap().token, tokens[9]);
    state.trash_note("thread", &author).unwrap();
    let entry = state.restore_from_trash("thread").unwrap();
    assert_eq!(entry.content, "9");
    state.record_version("thread", "agent_thread", &author, &entry.content).unwrap();
    assert_eq!(state.history("thread").unwrap().len(), 3);
}

#[test]
fn preview_and_apply_only_old_threads_and_preserve_pins() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let author = Author::Token("test".into());
    // Seed pre-retention history without a destructive migration.
    for id in ["thread", "trashed", "other"] {
        for _ in 0..10 {
            state.record_version(id, "email", &author, "é").unwrap();
        }
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("UPDATE versions SET note_type = 'agent_thread' WHERE note_id != 'other'", []).unwrap();
    let pinned = state.history("thread").unwrap()[0].token.clone();
    let (device, _) = state.pair_device(&state::NewDevice {
        name: "test", platform: "android", capabilities: "[]", settings: "{}",
    }).unwrap();
    state.set_device_bases(&device.id, &[("thread".into(), pinned.clone())]).unwrap();
    state.trash_note("trashed", &author).unwrap();
    let before = state.history("thread").unwrap();
    let report = state.prune_thread_history(3, false).unwrap();
    assert_eq!(report.iter().map(|r| r.rows).sum::<u64>(), 13);
    assert_eq!(report.iter().map(|r| r.bytes).sum::<u64>(), 26);
    assert_eq!(state.history("thread").unwrap(), before);
    assert_eq!(state.prune_thread_history(3, true).unwrap(), report);
    assert_eq!(state.history("thread").unwrap().len(), 4);
    assert_eq!(state.history("trashed").unwrap().len(), 3);
    assert_eq!(state.history("other").unwrap().len(), 10);
    assert!(state.version("thread", &pinned).unwrap().is_some());
    state.record_distinct_version("thread", "agent_thread", &author, "new").unwrap();
    assert_eq!(state.history("thread").unwrap().len(), 4);
    assert!(state.version("thread", &pinned).unwrap().is_some());
    assert_eq!(state.restore_from_trash("trashed").unwrap().content, "é");
    conn.execute("DELETE FROM device_bases", []).unwrap();
    state.record_version("thread", "agent_thread", &author, "last").unwrap();
    assert_eq!(state.history("thread").unwrap().len(), 3);
    assert!(state.prune_thread_history(0, true).is_err());
}
