use rusqlite::{Connection, params};
use state::{Author, State};
use tempfile::TempDir;

#[test]
fn mixed_legacy_new_restore_and_restart() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let author = Author::Token("test".into());
    let legacy = "old\r\n雪\0bytes";
    let old = state
        .record_distinct_version("note", "task", &author, legacy)
        .unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE versions SET content=?1, content_bytes=NULL WHERE token=?2",
        params![legacy, old.token],
    )
    .unwrap();
    let large = "new\r\n雪\0".repeat(200_000);
    let new = state
        .record_distinct_version("note", "task", &author, &large)
        .unwrap();
    let mut devices = Vec::new();
    for token in [&old.token, &new.token] {
        let (device, _) = state
            .pair_device(&state::NewDevice {
                name: "fixture",
                platform: "android",
                capabilities: "[]",
                settings: "{}",
            })
            .unwrap();
        state
            .set_device_bases(&device.id, &[("note".into(), token.clone())])
            .unwrap();
        devices.push(device.id);
    }
    assert_eq!(
        state
            .prune(&state::PrunePolicy {
                keep_versions: Some(1),
                max_age_ms: None
            })
            .unwrap(),
        0
    );
    for (token, content) in [(&old.token, legacy), (&new.token, large.as_str())] {
        assert_eq!(
            state
                .version("note", token)
                .unwrap()
                .unwrap()
                .content
                .as_bytes(),
            content.as_bytes()
        );
    }
    state.trash_note("note", &author).unwrap();
    assert_eq!(state.restore_from_trash("note").unwrap().content, large);
    drop(state);
    let state = State::open(&path, None).unwrap();
    for (device, token) in devices.iter().zip([&old.token, &new.token]) {
        assert_eq!(
            state.device_bases(device).unwrap(),
            vec![("note".into(), token.clone())]
        );
    }
    assert_eq!(state.history("note").unwrap(), vec![old, new]);
    // Restoring legacy content also remains exact.
    state
        .record_distinct_version("note", "task", &author, legacy)
        .unwrap();
    state.trash_note("note", &author).unwrap();
    assert_eq!(state.restore_from_trash("note").unwrap().content, legacy);
}

#[test]
fn corruption_refuses_before_trash_or_head_changes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let author = Author::Token("test".into());
    let version = state
        .record_version("note", "task", &author, "original")
        .unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE versions SET content=x'56485302'", [])
        .unwrap();
    assert!(
        state
            .history("note")
            .unwrap_err()
            .to_string()
            .contains("corrupt history snapshot")
    );
    assert!(state.trash_note("note", &author).is_err());
    assert!(!state.in_trash("note").unwrap());
    assert_eq!(
        state.current_version("note").unwrap().unwrap(),
        version.token
    );
}

#[test]
fn interrupted_write_rolls_back_on_restart() {
    if let Ok(path) = std::env::var("TV262_CRASH_DB") {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "BEGIN IMMEDIATE; UPDATE versions SET content=x'00'; UPDATE heads SET token='partial';",
        )
        .unwrap();
        // Exit with an open SQLite transaction, without running destructors.
        std::process::exit(0);
    }
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let original = state
        .record_version(
            "note",
            "task",
            &Author::Token("test".into()),
            &"exact".repeat(10000),
        )
        .unwrap();
    drop(state);
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "compressed_history::interrupted_write_rolls_back_on_restart"
            ])
            .env("TV262_CRASH_DB", &path)
            .status()
            .unwrap()
            .success()
    );
    let state = State::open(&path, None).unwrap();
    assert_eq!(state.head("note").unwrap().unwrap(), original);
}

#[test]
fn migration_preserves_legacy_rows() {
    if let Ok(path) = std::env::var("TV262_MIGRATION_DB") {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE; ALTER TABLE versions ADD COLUMN content_bytes INTEGER; PRAGMA user_version=32;").unwrap();
        std::process::exit(0);
    }
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = State::open(&path, None).unwrap();
    let version = state
        .record_version("note", "task", &Author::Token("test".into()), "legacy")
        .unwrap();
    drop(state);
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE versions SET content='legacy'", [])
        .unwrap();
    conn.execute_batch("ALTER TABLE versions DROP COLUMN content_bytes; PRAGMA user_version=31;")
        .unwrap();
    drop(conn);
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "compressed_history::migration_preserves_legacy_rows"])
            .env("TV262_MIGRATION_DB", &path)
            .status()
            .unwrap()
            .success()
    );
    let state = State::open(&path, None).unwrap();
    assert_eq!(state.head("note").unwrap().unwrap(), version);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT typeof(content) FROM versions", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "text"
    );
}
