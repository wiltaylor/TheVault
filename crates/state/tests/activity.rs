//! The activity log (activity Y1, Y6).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::json;
use state::{
    ACTIVITY_MAX_SUMMARY, ActivityFilter, Author, Clock, LiveEvent, NewActivity, NewDevice, State,
};
use tempfile::TempDir;

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

fn open(dir: &TempDir, clock: Arc<TestClock>) -> State {
    State::open_with_clock(&dir.path().join("state.db"), None, clock).unwrap()
}

fn owner() -> Author {
    Author::Session("owner".into())
}

fn page(state: &State, f: ActivityFilter) -> Vec<String> {
    state
        .activity(&f)
        .unwrap()
        .items
        .into_iter()
        .map(|a| a.summary)
        .collect()
}

#[test]
fn records_an_entry_and_publishes_it() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(1_000_000);
    let state = open(&dir, clock.clone());
    let mut sub = state.events().subscribe();
    let a = state
        .record_activity(
            NewActivity::new("note.create", "notes", &owner(), "Created memo \"Plans\"")
                .subject("note", "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA")
                .detail(json!({ "type": "memo", "version": "v1" })),
        )
        .unwrap();
    assert_eq!(a.at_ms, 1_000_000);
    assert_eq!(a.kind, "note.create");
    assert_eq!(a.category, "notes");
    assert_eq!(
        (a.actor_kind.as_str(), a.actor.as_str()),
        ("session", "owner")
    );
    assert_eq!(a.subject_kind.as_deref(), Some("note"));
    assert_eq!(a.detail["type"], "memo");
    let (_, event) = sub.try_recv().unwrap().unwrap();
    assert_eq!(event, LiveEvent::Activity { item: a.clone() });
    assert_eq!(event.name(), "activity");
    let got = state.activity(&ActivityFilter::default()).unwrap();
    assert_eq!(got.items, vec![a]);
    assert_eq!(got.next, None);
}

#[test]
fn summaries_are_one_bounded_line_and_detail_is_bounded() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir, TestClock::new(1));
    let long = format!("line one\nline two {}", "x".repeat(1000));
    let a = state
        .record_activity(
            NewActivity::server("ai.tool", "ai", long).detail(json!({ "big": "y".repeat(10_000) })),
        )
        .unwrap();
    assert!(!a.summary.contains('\n'));
    assert!(a.summary.starts_with("line one line two"));
    assert_eq!(a.summary.chars().count(), ACTIVITY_MAX_SUMMARY);
    assert!(a.detail.to_string().len() <= 4096);
}

#[test]
fn pages_newest_first_with_before() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(1_000);
    let state = open(&dir, clock.clone());
    for i in 0..5 {
        state.record_activity(NewActivity::server("x.y", "sync", format!("e{i}")));
        clock.advance(1);
    }
    let first = state
        .activity(&ActivityFilter {
            limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    let names: Vec<_> = first.items.iter().map(|a| a.summary.as_str()).collect();
    assert_eq!(names, ["e4", "e3"]);
    let second = state
        .activity(&ActivityFilter {
            limit: Some(2),
            before: first.next.clone(),
            ..Default::default()
        })
        .unwrap();
    let names: Vec<_> = second.items.iter().map(|a| a.summary.as_str()).collect();
    assert_eq!(names, ["e2", "e1"]);
    let third = state
        .activity(&ActivityFilter {
            limit: Some(2),
            before: second.next,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(third.items.len(), 1);
    assert_eq!(third.next, None);
}

#[test]
fn filters_by_category_actor_kind_and_text() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir, TestClock::new(1_000));
    let agent = Author::Agent("researcher".into());
    state.record_activity(NewActivity::new(
        "ai.turn.start",
        "ai",
        &agent,
        "Turn started",
    ));
    state.record_activity(NewActivity::new(
        "ai.tool",
        "ai",
        &agent,
        "search_notes: 50% off",
    ));
    state.record_activity(NewActivity::new(
        "note.update",
        "notes",
        &owner(),
        "Edited Plans",
    ));
    state.record_activity(NewActivity::new(
        "ai.turn.end",
        "ai",
        &Author::Agent("other".into()),
        "Turn ended",
    ));
    let f = |category: Option<&str>, actor: Option<&str>, kind: Option<&str>, q: Option<&str>| {
        page(
            &state,
            ActivityFilter {
                category: category.map(Into::into),
                actor: actor.map(Into::into),
                kind: kind.map(Into::into),
                q: q.map(Into::into),
                ..Default::default()
            },
        )
    };
    assert_eq!(f(Some("notes"), None, None, None), ["Edited Plans"]);
    assert_eq!(
        f(Some("ai"), Some("agent:researcher"), None, None),
        ["search_notes: 50% off", "Turn started"]
    );
    assert_eq!(f(None, Some("agent"), None, None).len(), 3);
    assert_eq!(f(None, Some("session"), None, None), ["Edited Plans"]);
    assert_eq!(
        f(None, None, Some("ai.turn"), None),
        ["Turn ended", "Turn started"]
    );
    assert_eq!(
        f(None, None, Some("ai.tool"), None),
        ["search_notes: 50% off"]
    );
    // `ai.t` is not a dotted prefix of `ai.tool`.
    assert!(f(None, None, Some("ai.t"), None).is_empty());
    assert_eq!(f(None, None, None, Some("PLANS")), ["Edited Plans"]);
    // LIKE's wildcards match only themselves.
    assert_eq!(f(None, None, None, Some("50%")), ["search_notes: 50% off"]);
    assert_eq!(f(None, None, None, Some("_")), ["search_notes: 50% off"]);
}

#[test]
fn coalesces_one_actors_edits_of_one_note_within_a_minute() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(1_000_000);
    let state = open(&dir, clock.clone());
    let edit = |who: &Author, v: &str| {
        state
            .record_activity(
                NewActivity::new("note.update", "notes", who, format!("Edited Plans ({v})"))
                    .subject("note", "N1")
                    .detail(json!({ "version": v }))
                    .coalescing(),
            )
            .unwrap()
    };
    let first = edit(&owner(), "v1");
    clock.advance(10_000);
    let second = edit(&owner(), "v2");
    assert_eq!(second.id, first.id);
    assert_eq!(second.detail["edits"], 2);
    assert_eq!(second.detail["version"], "v2");
    clock.advance(10_000);
    // Another actor's edit is an entry of its own, and ends the burst.
    let agent = edit(&Author::Agent("helper".into()), "v3");
    assert_ne!(agent.id, first.id);
    clock.advance(1_000);
    let fourth = edit(&owner(), "v4");
    assert_ne!(fourth.id, first.id);
    clock.advance(61_000);
    let fifth = edit(&owner(), "v5");
    assert_ne!(fifth.id, fourth.id);
    let all = state.activity(&ActivityFilter::default()).unwrap().items;
    assert_eq!(all.len(), 4);
    assert_eq!(all[3].summary, "Edited Plans (v2)");
}

#[test]
fn a_device_token_is_recorded_as_the_device() {
    let dir = TempDir::new().unwrap();
    let state = open(&dir, TestClock::new(1_000));
    let (device, _) = state
        .pair_device(&NewDevice {
            name: "Pixel",
            platform: "android",
            capabilities: "[]",
            settings: "{}",
        })
        .unwrap();
    let a = state
        .record_activity(NewActivity::new(
            "sync.round",
            "sync",
            &Author::Token(device.token_id.clone()),
            "Synced",
        ))
        .unwrap();
    assert_eq!(
        (a.actor_kind.as_str(), a.actor.as_str()),
        ("device", "Pixel")
    );
}

#[test]
fn prunes_by_age_and_count() {
    let dir = TempDir::new().unwrap();
    let clock = TestClock::new(DAY_MS);
    let state = open(&dir, clock.clone());
    state.record_activity(NewActivity::server("a.old", "sync", "old"));
    clock.advance(80 * DAY_MS);
    state.record_activity(NewActivity::server("a.mid", "sync", "mid"));
    clock.advance(11 * DAY_MS);
    // The first record of a new day prunes: "old" is now 91 days old.
    state.record_activity(NewActivity::server("a.new", "sync", "new"));
    assert_eq!(page(&state, ActivityFilter::default()), ["new", "mid"]);
    for i in 0..5 {
        state.record_activity(NewActivity::server("a.n", "sync", format!("n{i}")));
    }
    assert_eq!(
        state
            .prune_activity_to(state::ACTIVITY_RETENTION_MS, 3)
            .unwrap(),
        4
    );
    assert_eq!(page(&state, ActivityFilter::default()), ["n4", "n3", "n2"]);
    drop(state);
    // Opening leaves the log as it is: the server prunes it as it starts.
    clock.advance(100 * DAY_MS);
    let state = open(&dir, clock);
    assert_eq!(page(&state, ActivityFilter::default()).len(), 3);
    assert_eq!(state.prune_activity().unwrap(), 3);
    assert!(page(&state, ActivityFilter::default()).is_empty());
}

#[test]
fn recording_never_fails_its_caller() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    let state = open(&dir, TestClock::new(1_000));
    // Break the table from outside: the record is dropped, not an error.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE activity;").unwrap();
    drop(conn);
    assert_eq!(
        state.record_activity(NewActivity::server("x.y", "sync", "lost")),
        None
    );
    assert!(state.activity(&ActivityFilter::default()).is_err());
}

#[test]
fn migrates_an_older_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    drop(open(&dir, TestClock::new(1)));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE activity; PRAGMA user_version = 16;")
            .unwrap();
    }
    let state = open(&dir, TestClock::new(2));
    assert!(
        state
            .record_activity(NewActivity::server("x.y", "sync", "back"))
            .is_some()
    );
    drop(state);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 17);
}
