//! Paired devices and pairing codes in `state.db` (D2, D3).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use rusqlite::Connection;
use state::{Clock, Error, NewDevice, PushTarget, Scope, State};
use tempfile::TempDir;

struct TestClock(AtomicI64);

impl TestClock {
    fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn open(dir: &TempDir) -> (State, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();
    (state, clock)
}

fn phone() -> NewDevice<'static> {
    NewDevice {
        name: "Pixel",
        platform: "android",
        capabilities: r#"["sms"]"#,
        settings: "{}",
    }
}

#[test]
fn pairing_makes_a_write_token_bound_to_the_device() {
    let dir = TempDir::new().unwrap();
    let (state, clock) = open(&dir);
    let (device, secret) = state.pair_device(&phone()).unwrap();
    assert_eq!(device.name, "Pixel");
    assert_eq!(device.platform, "android");
    assert_eq!(device.capabilities, r#"["sms"]"#);
    assert_eq!(
        device.token_id,
        format!("device-{}", device.id.to_lowercase())
    );
    assert_eq!(device.last_seen_ms, None);
    assert_eq!(device.push, None);

    clock.advance(500);
    let token = state.authenticate_token(&secret).unwrap().unwrap();
    assert_eq!(token.name, device.token_id);
    assert_eq!(token.scope, Scope::Write);
    assert_eq!(
        state.device(&device.id).unwrap().last_seen_ms,
        Some(1_000_500)
    );
    assert_eq!(
        state
            .device_for_token(&device.token_id)
            .unwrap()
            .unwrap()
            .id,
        device.id
    );
    assert_eq!(state.device_for_token("admin").unwrap(), None);
}

#[test]
fn an_unknown_platform_is_refused_and_leaves_no_token() {
    let dir = TempDir::new().unwrap();
    let (state, _) = open(&dir);
    let err = state
        .pair_device(&NewDevice {
            platform: "windows",
            ..phone()
        })
        .unwrap_err();
    assert!(matches!(err, Error::UnknownPlatform(_)), "{err}");
    assert!(state.list_tokens().unwrap().is_empty());
}

#[test]
fn devices_list_rename_and_change_settings() {
    let dir = TempDir::new().unwrap();
    let (state, clock) = open(&dir);
    let (a, _) = state.pair_device(&phone()).unwrap();
    clock.advance(1);
    let (b, _) = state
        .pair_device(&NewDevice {
            name: "Laptop",
            platform: "linux",
            capabilities: "[]",
            settings: "{}",
        })
        .unwrap();
    let ids: Vec<String> = state
        .list_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(ids, [a.id.clone(), b.id.clone()]);

    let renamed = state.update_device(&a.id, Some("Old phone"), None).unwrap();
    assert_eq!(renamed.name, "Old phone");
    assert_eq!(renamed.settings, "{}");
    let set = state
        .update_device(&a.id, None, Some(r#"{"sms_send":true}"#))
        .unwrap();
    assert_eq!(set.name, "Old phone");
    assert_eq!(set.settings, r#"{"sms_send":true}"#);
    assert!(matches!(
        state.update_device("nope", Some("x"), None),
        Err(Error::UnknownDevice(_))
    ));
}

#[test]
fn revoking_deletes_the_token_and_the_push_endpoint() {
    let dir = TempDir::new().unwrap();
    let (state, _) = open(&dir);
    let (device, secret) = state.pair_device(&phone()).unwrap();
    let push = PushTarget {
        endpoint: "https://push.example/up/abc".into(),
        p256dh: "key".into(),
        auth: "auth".into(),
    };
    assert_eq!(
        state.set_device_push(&device.id, Some(&push)).unwrap().push,
        Some(push)
    );
    state.revoke_device(&device.id).unwrap();
    assert_eq!(state.authenticate_token(&secret).unwrap(), None);
    assert!(state.list_devices().unwrap().is_empty());
    assert!(matches!(
        state.revoke_device(&device.id),
        Err(Error::UnknownDevice(_))
    ));
}

#[test]
fn revoking_the_token_directly_removes_the_device() {
    let dir = TempDir::new().unwrap();
    let (state, _) = open(&dir);
    let (device, _) = state.pair_device(&phone()).unwrap();
    state.revoke_token(&device.token_id).unwrap();
    assert!(state.list_devices().unwrap().is_empty());
}

#[test]
fn clearing_push_removes_the_endpoint() {
    let dir = TempDir::new().unwrap();
    let (state, _) = open(&dir);
    let (device, _) = state.pair_device(&phone()).unwrap();
    let push = PushTarget {
        endpoint: "https://push.example/up/abc".into(),
        p256dh: "key".into(),
        auth: "auth".into(),
    };
    state.set_device_push(&device.id, Some(&push)).unwrap();
    assert_eq!(state.set_device_push(&device.id, None).unwrap().push, None);
}

#[test]
fn a_pairing_code_works_once() {
    let dir = TempDir::new().unwrap();
    let (state, _) = open(&dir);
    let code = state.create_pair_code(60_000).unwrap();
    assert_eq!(code.expires_ms, 1_060_000);
    assert_eq!(code.code.len(), 9);
    assert_eq!(&code.code[4..5], "-");
    assert!(!state.take_pair_code("WRONG-CODE").unwrap());
    // Typed in lower case without the hyphen still matches.
    let typed = code.code.replace('-', "").to_lowercase();
    assert!(state.take_pair_code(&typed).unwrap());
    assert!(!state.take_pair_code(&code.code).unwrap());
}

#[test]
fn a_pairing_code_expires() {
    let dir = TempDir::new().unwrap();
    let (state, clock) = open(&dir);
    let code = state.create_pair_code(60_000).unwrap();
    clock.advance(60_000);
    assert!(!state.take_pair_code(&code.code).unwrap());
}

#[test]
fn a_version_7_database_gains_the_devices_tables() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    {
        let state = State::open(&path, None).unwrap();
        state.create_token("old", Scope::Read).unwrap();
    }
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE devices; DROP TABLE pair_codes; PRAGMA user_version = 7;")
            .unwrap();
    }
    let state = State::open(&path, None).unwrap();
    assert_eq!(state.list_tokens().unwrap().len(), 1);
    state.pair_device(&phone()).unwrap();
    assert_eq!(state.list_devices().unwrap().len(), 1);
    drop(state);
    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 8);
}
