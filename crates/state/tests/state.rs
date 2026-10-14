use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use state::{
    Author, Clock, Error, JobStatus, PrunePolicy, RunOutcome, Scope, SecretKey, State,
    key_file_path,
};
use tempfile::TempDir;

const NOTE_A: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const NOTE_B: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

struct TestClock(AtomicI64);

impl TestClock {
    fn new(start: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(start)))
    }
    fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn open(dir: &TempDir) -> State {
    State::open(&dir.path().join("state.db"), None).unwrap()
}

fn open_with_key(dir: &TempDir, key: SecretKey) -> State {
    State::open(&dir.path().join("state.db"), Some(key)).unwrap()
}

#[test]
fn history_returns_every_version_in_order_with_authors() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(1_000);
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();

    let v1 = state
        .record_version(
            NOTE_A,
            "task",
            &Author::Session("browser".into()),
            "title = \"one\"\n",
        )
        .unwrap();
    clock.advance(10);
    let v2 = state
        .record_version(
            NOTE_A,
            "task",
            &Author::Token("importer".into()),
            "title = \"two\"\n",
        )
        .unwrap();
    clock.advance(10);
    let v3 = state
        .record_version(
            NOTE_A,
            "task",
            &Author::Handler("compute_due".into()),
            "title = \"three\"\n",
        )
        .unwrap();
    state
        .record_version(NOTE_B, "task", &Author::Session("browser".into()), "other")
        .unwrap();

    let history = state.history(NOTE_A).unwrap();
    assert_eq!(history.len(), 3);
    let tokens: Vec<_> = history.iter().map(|v| v.token.clone()).collect();
    assert_eq!(tokens, vec![v1.token, v2.token, v3.token.clone()]);
    let authors: Vec<_> = history.iter().map(|v| v.author.clone()).collect();
    assert_eq!(
        authors,
        vec![
            Author::Session("browser".into()),
            Author::Token("importer".into()),
            Author::Handler("compute_due".into()),
        ]
    );
    let contents: Vec<_> = history.iter().map(|v| v.content.as_str()).collect();
    assert_eq!(
        contents,
        vec![
            "title = \"one\"\n",
            "title = \"two\"\n",
            "title = \"three\"\n"
        ]
    );
    assert_eq!(
        history.iter().map(|v| v.at_ms).collect::<Vec<_>>(),
        vec![1_000, 1_010, 1_020]
    );

    assert_eq!(
        state.current_version(NOTE_A).unwrap(),
        Some(v3.token.clone())
    );
    let earlier = state.version(NOTE_A, &history[0].token).unwrap().unwrap();
    assert_eq!(earlier.content, "title = \"one\"\n");
    assert!(state.version(NOTE_A, "no-such-version").unwrap().is_none());
    assert!(
        state
            .history("01JB8Z4K2MQ7V3X9T5R6W8Y2ZZ")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn version_tokens_are_unique_per_write() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let author = Author::Session("s".into());
    let a = state
        .record_version(NOTE_A, "task", &author, "same")
        .unwrap();
    let b = state
        .record_version(NOTE_A, "task", &author, "same")
        .unwrap();
    assert_ne!(a.token, b.token);
}

#[test]
fn head_is_the_current_version_of_each_live_note() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let author = Author::Session("s".into());
    assert_eq!(state.head(NOTE_A).unwrap(), None);
    state
        .record_version(NOTE_A, "task", &author, "first")
        .unwrap();
    let last = state
        .record_version(NOTE_A, "task", &author, "last")
        .unwrap();
    state.record_version(NOTE_B, "task", &author, "b").unwrap();

    assert_eq!(state.head(NOTE_A).unwrap(), Some(last));
    assert_eq!(state.live_notes().unwrap(), vec![NOTE_A, NOTE_B]);

    state.trash_note(NOTE_B, &author).unwrap();
    assert_eq!(state.head(NOTE_B).unwrap(), None);
    assert_eq!(state.live_notes().unwrap(), vec![NOTE_A]);
}

#[test]
fn history_survives_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let state = open(&dir);
        state
            .record_version(NOTE_A, "task", &Author::Session("s".into()), "kept")
            .unwrap();
    }
    let state = open(&dir);
    assert_eq!(state.history(NOTE_A).unwrap()[0].content, "kept");
}

#[test]
fn restoring_a_trashed_note_returns_its_last_content() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let author = Author::Session("browser".into());
    state
        .record_version(NOTE_A, "task", &author, "first")
        .unwrap();
    state
        .record_version(NOTE_A, "task", &author, "last")
        .unwrap();

    state.trash_note(NOTE_A, &author).unwrap();
    let trash = state.list_trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].note_id, NOTE_A);
    assert_eq!(trash[0].note_type, "task");
    assert_eq!(trash[0].deleted_by, author);
    assert_eq!(state.current_version(NOTE_A).unwrap(), None);

    let restored = state.restore_from_trash(NOTE_A).unwrap();
    assert_eq!(restored.content, "last");
    assert_eq!(restored.note_type, "task");
    assert!(state.list_trash().unwrap().is_empty());
    assert!(matches!(
        state.restore_from_trash(NOTE_A),
        Err(Error::NotInTrash(id)) if id == NOTE_A
    ));
}

#[test]
fn trashing_a_note_without_history_fails() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert!(matches!(
        state.trash_note(NOTE_A, &Author::Session("s".into())),
        Err(Error::UnknownNote(_))
    ));
}

#[test]
fn pruning_by_count_keeps_exactly_n_versions() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let author = Author::Token("agent".into());
    for i in 0..7 {
        state
            .record_version(NOTE_A, "task", &author, &format!("a{i}"))
            .unwrap();
    }
    for i in 0..2 {
        state
            .record_version(NOTE_B, "task", &author, &format!("b{i}"))
            .unwrap();
    }

    let removed = state
        .prune(&PrunePolicy {
            keep_versions: Some(3),
            max_age_ms: None,
        })
        .unwrap();
    assert_eq!(removed, 4);
    let a: Vec<_> = state
        .history(NOTE_A)
        .unwrap()
        .into_iter()
        .map(|v| v.content)
        .collect();
    assert_eq!(a, vec!["a4", "a5", "a6"]);
    assert_eq!(state.history(NOTE_B).unwrap().len(), 2);
}

#[test]
fn pruning_is_off_by_default() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let author = Author::Token("agent".into());
    for i in 0..5 {
        state
            .record_version(NOTE_A, "task", &author, &format!("{i}"))
            .unwrap();
    }
    assert_eq!(state.prune(&PrunePolicy::default()).unwrap(), 0);
    assert_eq!(state.history(NOTE_A).unwrap().len(), 5);
}

#[test]
fn pruning_keep_zero_is_rejected() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert!(matches!(
        state.prune(&PrunePolicy {
            keep_versions: Some(0),
            max_age_ms: None
        }),
        Err(Error::InvalidPrunePolicy(_))
    ));
}

#[test]
fn pruning_by_age_keeps_the_current_version() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(0);
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();
    let author = Author::Token("agent".into());
    state
        .record_version(NOTE_A, "task", &author, "old1")
        .unwrap();
    state
        .record_version(NOTE_A, "task", &author, "old2")
        .unwrap();
    state
        .record_version(NOTE_B, "task", &author, "only")
        .unwrap();
    clock.advance(40 * DAY_MS);
    state
        .record_version(NOTE_A, "task", &author, "new")
        .unwrap();
    clock.advance(DAY_MS);

    let removed = state
        .prune(&PrunePolicy {
            keep_versions: None,
            max_age_ms: Some(30 * DAY_MS),
        })
        .unwrap();
    assert_eq!(removed, 2);
    let a: Vec<_> = state
        .history(NOTE_A)
        .unwrap()
        .into_iter()
        .map(|v| v.content)
        .collect();
    assert_eq!(a, vec!["new"]);
    let b: Vec<_> = state
        .history(NOTE_B)
        .unwrap()
        .into_iter()
        .map(|v| v.content)
        .collect();
    assert_eq!(b, vec!["only"]);
}

#[test]
fn tokens_authenticate_by_secret_and_record_last_use() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(5_000);
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();

    let (info, secret) = state.create_token("agent", Scope::Write).unwrap();
    assert_eq!(info.name, "agent");
    assert_eq!(info.scope, Scope::Write);
    assert_eq!(info.created_ms, 5_000);
    assert_eq!(info.last_used_ms, None);

    clock.advance(100);
    let found = state.authenticate_token(&secret).unwrap().unwrap();
    assert_eq!(found.name, "agent");
    assert_eq!(found.last_used_ms, Some(5_100));
    assert!(state.authenticate_token("tv_wrong").unwrap().is_none());

    assert!(matches!(
        state.create_token("agent", Scope::Read),
        Err(Error::TokenExists(name)) if name == "agent"
    ));

    let listed = state.list_tokens().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].last_used_ms, Some(5_100));

    state.revoke_token("agent").unwrap();
    assert!(state.authenticate_token(&secret).unwrap().is_none());
    assert!(matches!(
        state.revoke_token("agent"),
        Err(Error::UnknownToken(_))
    ));

    drop(state);
    let bytes = read_db_bytes(&dir);
    assert!(
        !contains(&bytes, secret.as_bytes()),
        "token secret stored in plaintext"
    );
}

#[test]
fn scopes_are_ordered() {
    assert!(Scope::Admin.allows(Scope::Write));
    assert!(Scope::Admin.allows(Scope::Read));
    assert!(Scope::Write.allows(Scope::Read));
    assert!(!Scope::Write.allows(Scope::Admin));
    assert!(!Scope::Read.allows(Scope::Write));
    assert_eq!("admin".parse::<Scope>().unwrap(), Scope::Admin);
    assert_eq!(Scope::Read.as_str(), "read");
    assert!("root".parse::<Scope>().is_err());
}

#[test]
fn password_is_stored_as_argon2id_hash() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    assert!(!state.has_password().unwrap());
    assert!(!state.verify_password("anything").unwrap());

    state.set_password("correct horse battery staple").unwrap();
    assert!(state.has_password().unwrap());
    assert!(
        state
            .verify_password("correct horse battery staple")
            .unwrap()
    );
    assert!(!state.verify_password("wrong").unwrap());

    state.set_password("changed").unwrap();
    assert!(
        !state
            .verify_password("correct horse battery staple")
            .unwrap()
    );
    assert!(state.verify_password("changed").unwrap());

    drop(state);
    let bytes = read_db_bytes(&dir);
    assert!(contains(&bytes, b"$argon2id$"));
    assert!(!contains(&bytes, b"changed\0") && !contains(&bytes, b"correct horse"));
}

#[test]
fn secrets_round_trip_and_list_without_values() {
    let dir = TempDir::new().unwrap();
    let state = open_with_key(&dir, SecretKey::generate());
    state.set_secret("github", "ghp_value_one").unwrap();
    state.set_secret("weather", "wx-key").unwrap();
    state.set_secret("github", "ghp_value_two").unwrap();

    assert_eq!(
        state.get_secret("github").unwrap().as_deref(),
        Some("ghp_value_two")
    );
    assert_eq!(state.get_secret("missing").unwrap(), None);
    let names: Vec<_> = state
        .list_secrets()
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, vec!["github", "weather"]);

    state.delete_secret("weather").unwrap();
    assert_eq!(state.get_secret("weather").unwrap(), None);
    assert!(matches!(
        state.delete_secret("weather"),
        Err(Error::UnknownSecret(_))
    ));
}

#[test]
fn secret_plaintext_never_appears_in_state_db_bytes() {
    let dir = TempDir::new().unwrap();
    let plaintext = "plaintext-marker-7f3a9c2e-do-not-store";
    {
        let state = open_with_key(&dir, SecretKey::generate());
        state.set_secret("api", plaintext).unwrap();
        assert_eq!(state.get_secret("api").unwrap().as_deref(), Some(plaintext));
    }
    let bytes = read_db_bytes(&dir);
    assert!(contains(&bytes, b"api"), "secret name should be stored");
    assert!(
        !contains(&bytes, plaintext.as_bytes()),
        "secret plaintext found in state.db"
    );
}

#[test]
fn secrets_cannot_be_read_with_the_wrong_key() {
    let dir = TempDir::new().unwrap();
    {
        let state = open_with_key(&dir, SecretKey::generate());
        state.set_secret("api", "s3cret").unwrap();
    }
    let state = open_with_key(&dir, SecretKey::generate());
    let err = state.get_secret("api").unwrap_err();
    assert!(matches!(err, Error::WrongKey { .. }), "{err:?}");
    assert!(err.to_string().contains("key is wrong"), "{err}");
}

#[test]
fn secrets_cannot_be_read_without_a_key() {
    let dir = TempDir::new().unwrap();
    {
        let state = open_with_key(&dir, SecretKey::generate());
        state.set_secret("api", "s3cret").unwrap();
    }
    let state = open(&dir);
    let err = state.get_secret("api").unwrap_err();
    assert!(matches!(err, Error::KeyMissing { .. }), "{err:?}");
    assert!(err.to_string().contains("key is missing"), "{err}");
    assert!(matches!(
        state.set_secret("x", "y"),
        Err(Error::KeyMissing { .. })
    ));
    // Names are still listable without the key.
    assert_eq!(state.list_secrets().unwrap()[0].name, "api");
}

#[test]
fn key_files_are_written_once_and_loaded() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config/thevault/vault.key");
    let key = SecretKey::create_file(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap().len(), 32);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(matches!(SecretKey::create_file(&path), Err(Error::Io(_))));

    // A state opened with the loaded key reads secrets written with the created one.
    {
        let state = open_with_key(&dir, key);
        state.set_secret("api", "v").unwrap();
    }
    let state = open_with_key(&dir, SecretKey::load_file(&path).unwrap());
    assert_eq!(state.get_secret("api").unwrap().as_deref(), Some("v"));
}

#[test]
fn loading_a_missing_or_malformed_key_file_says_so() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nope.key");
    let err = SecretKey::load_file(&missing).unwrap_err();
    assert!(
        matches!(err, Error::KeyMissing { path: Some(_) }),
        "{err:?}"
    );
    assert!(err.to_string().contains("key is missing"), "{err}");
    assert!(err.to_string().contains("nope.key"), "{err}");

    let short = dir.path().join("short.key");
    std::fs::write(&short, [0u8; 16]).unwrap();
    let err = SecretKey::load_file(&short).unwrap_err();
    assert!(matches!(err, Error::WrongKey { .. }), "{err:?}");
    assert!(err.to_string().contains("key is wrong"), "{err}");
}

#[test]
fn key_file_path_follows_override_then_xdg_then_home() {
    let p = key_file_path(
        "V1",
        Some("/tmp/explicit.key".into()),
        Some("/xdg".into()),
        Some("/home/u".into()),
    );
    assert_eq!(p.unwrap(), std::path::PathBuf::from("/tmp/explicit.key"));
    let p = key_file_path("V1", None, Some("/xdg".into()), Some("/home/u".into()));
    assert_eq!(p.unwrap(), std::path::PathBuf::from("/xdg/thevault/V1.key"));
    let p = key_file_path("V1", None, None, Some("/home/u".into()));
    assert_eq!(
        p.unwrap(),
        std::path::PathBuf::from("/home/u/.config/thevault/V1.key")
    );
    let p = key_file_path(
        "V1",
        Some("".into()),
        Some("".into()),
        Some("/home/u".into()),
    );
    assert_eq!(
        p.unwrap(),
        std::path::PathBuf::from("/home/u/.config/thevault/V1.key")
    );
    assert!(key_file_path("V1", None, None, None).is_none());
}

#[test]
fn task_runs_record_start_end_outcome_and_logs() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(1_000);
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();

    let first = state.start_run("sync", 1).unwrap();
    state.append_run_log(first, "fetching").unwrap();
    state.append_run_log(first, "got 3 items").unwrap();
    clock.advance(500);
    state
        .finish_run(first, &RunOutcome::Failed("timeout".into()))
        .unwrap();

    clock.advance(1_000);
    let second = state.start_run("sync", 2).unwrap();
    let running = state.run(second).unwrap().unwrap();
    assert_eq!(running.outcome, RunOutcome::Running);
    assert_eq!(running.ended_ms, None);
    clock.advance(20);
    state.finish_run(second, &RunOutcome::Succeeded).unwrap();
    state.start_run("other", 1).unwrap();

    let run = state.run(first).unwrap().unwrap();
    assert_eq!(run.task, "sync");
    assert_eq!(run.attempt, 1);
    assert_eq!(run.started_ms, 1_000);
    assert_eq!(run.ended_ms, Some(1_500));
    assert_eq!(run.outcome, RunOutcome::Failed("timeout".into()));
    assert_eq!(run.log, "fetching\ngot 3 items\n");

    let runs = state.runs("sync", 10).unwrap();
    assert_eq!(
        runs.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![second, first]
    );
    assert_eq!(state.runs("sync", 1).unwrap().len(), 1);
    assert_eq!(
        state.latest_run("sync").unwrap().unwrap().outcome,
        RunOutcome::Succeeded
    );
    assert!(state.latest_run("never").unwrap().is_none());

    assert!(matches!(
        state.finish_run(second, &RunOutcome::Succeeded),
        Err(Error::RunFinished(_))
    ));
    assert!(matches!(
        state.append_run_log(9999, "x"),
        Err(Error::UnknownRun(9999))
    ));
}

fn read_db_bytes(dir: &TempDir) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("state.db")
        {
            bytes.extend(std::fs::read(&path).unwrap());
        }
    }
    assert!(!bytes.is_empty());
    bytes
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn handler_jobs_stay_queued_until_they_finish_and_failures_are_kept() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let ids = state
        .queue_jobs(&["one".to_owned(), "two".to_owned()])
        .unwrap();
    assert_eq!(ids.len(), 2);
    drop(state);

    // The queue survives a restart.
    let state = open(&dir);
    let queued = state.queued_jobs().unwrap();
    let payloads: Vec<&str> = queued.iter().map(|j| j.payload.as_str()).collect();
    assert_eq!(payloads, ["one", "two"]);
    assert!(queued.iter().all(|j| j.status == JobStatus::Queued));

    state.finish_job(ids[0], Ok(()), "").unwrap();
    state
        .finish_job(ids[1], Err("it broke".into()), "a log line\n")
        .unwrap();
    assert!(state.queued_jobs().unwrap().is_empty());
    let failed = state.failed_jobs().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, JobStatus::Failed("it broke".into()));
    assert_eq!(failed[0].log, "a log line\n");
    assert!(matches!(
        state.finish_job(ids[1], Ok(()), ""),
        Err(Error::Corrupt(_))
    ));

    let refused = state.record_failed_job("three", "too deep").unwrap();
    assert_eq!(state.failed_jobs().unwrap()[1].id, refused);
    assert!(state.queued_jobs().unwrap().is_empty());
}

#[test]
fn a_version_1_state_db_gains_the_handler_queue() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = open(&dir);
    state
        .record_version(
            NOTE_A,
            "task",
            &Author::Session("s".into()),
            "title = \"A\"\n",
        )
        .unwrap();
    drop(state);
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE handler_jobs; PRAGMA user_version = 1;")
            .unwrap();
    }
    let state = open(&dir);
    assert_eq!(state.history(NOTE_A).unwrap().len(), 1);
    state.queue_jobs(&["job".to_owned()]).unwrap();
    assert_eq!(state.queued_jobs().unwrap().len(), 1);
}

#[test]
fn the_vault_id_is_made_once_and_kept() {
    let dir = TempDir::new().unwrap();
    let id = open(&dir).vault_id().unwrap();
    assert_eq!(id.len(), 32);
    assert_eq!(open(&dir).vault_id().unwrap(), id);
    let other = TempDir::new().unwrap();
    assert_ne!(open(&other).vault_id().unwrap(), id);
}

#[test]
fn a_backup_copy_holds_the_history_and_the_vault_id() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let id = state.vault_id().unwrap();
    state
        .record_version(
            NOTE_A,
            "task",
            &Author::Session("s".into()),
            "title = \"A\"\n",
        )
        .unwrap();
    let out = TempDir::new().unwrap();
    let copy = out.path().join("copy.db");
    state.backup_to(&copy).unwrap();
    let restored = State::open(&copy, None).unwrap();
    assert_eq!(restored.vault_id().unwrap(), id);
    assert_eq!(restored.history(NOTE_A).unwrap().len(), 1);
}

#[test]
fn rapid_browser_saves_merge_into_one_version_with_a_new_token() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(0);
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();
    let me = Author::Session("browser".into());

    let first = state.record_version(NOTE_A, "task", &me, "t").unwrap();
    clock.advance(1_500);
    let second = state.record_version(NOTE_A, "task", &me, "ty").unwrap();
    clock.advance(1_500);
    let third = state.record_version(NOTE_A, "task", &me, "typed").unwrap();

    let history = state.history(NOTE_A).unwrap();
    assert_eq!(history.len(), 1, "{history:?}");
    assert_eq!(history[0].content, "typed");
    assert_eq!(history[0].at_ms, 3_000);
    // Every save still moves the token, so a stale writer gets a conflict (D12).
    assert_ne!(first.token, second.token);
    assert_ne!(second.token, third.token);
    assert_eq!(state.current_version(NOTE_A).unwrap(), Some(third.token));
}

#[test]
fn a_pause_another_author_or_a_long_run_starts_a_new_version() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(0);
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();
    let me = Author::Session("browser".into());

    state.record_version(NOTE_A, "task", &me, "a").unwrap();
    clock.advance(state::MERGE_GAP_MS + 1);
    state.record_version(NOTE_A, "task", &me, "b").unwrap();
    assert_eq!(state.history(NOTE_A).unwrap().len(), 2, "a pause");

    clock.advance(1_000);
    state
        .record_version(NOTE_A, "task", &Author::Handler("count".into()), "c")
        .unwrap();
    clock.advance(1_000);
    state.record_version(NOTE_A, "task", &me, "d").unwrap();
    assert_eq!(
        state.history(NOTE_A).unwrap().len(),
        4,
        "another author in between"
    );

    // Saving every minute keeps merging until the run is MERGE_SPAN_MS old.
    let mut saves = 0;
    while saves * 60_000 <= state::MERGE_SPAN_MS {
        clock.advance(60_000);
        state
            .record_version(NOTE_A, "task", &me, &format!("e{saves}"))
            .unwrap();
        saves += 1;
    }
    assert_eq!(state.history(NOTE_A).unwrap().len(), 5, "a long run");
}

#[test]
fn saves_by_tokens_never_merge() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir);
    let agent = Author::Token("agent".into());
    for i in 0..3 {
        state
            .record_version(NOTE_A, "task", &agent, &format!("{i}"))
            .unwrap();
    }
    assert_eq!(state.history(NOTE_A).unwrap().len(), 3);
}
