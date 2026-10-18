//! Device commands in `state.db`: queueing, listing, reports, expiry and retention (D22).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use rusqlite::Connection;
use state::{
    COMMAND_RETENTION_MS, Clock, CommandStatus, Error, LiveEvent, NewCommand, NewDevice, State,
};
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

fn open(dir: &TempDir) -> (State, Arc<TestClock>, String) {
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let state = State::open_with_clock(&dir.path().join("state.db"), None, clock.clone()).unwrap();
    let (device, _) = state
        .pair_device(&NewDevice {
            name: "Pixel",
            platform: "android",
            capabilities: "[]",
            settings: "{}",
        })
        .unwrap();
    (state, clock, device.id)
}

fn sms<'a>(device: &'a str, ttl_ms: i64) -> NewCommand<'a> {
    NewCommand {
        device_id: device,
        name: "send_sms",
        payload: r#"{"to":"+61400000000","body":"hi"}"#,
        source: "task:remind",
        ttl_ms,
    }
}

#[test]
fn a_queued_command_is_pending_until_the_device_reports_on_it() {
    let dir = TempDir::new().unwrap();
    let (state, clock, device) = open(&dir);
    let mut events = state.events().subscribe();
    let first = state.queue_device_command(&sms(&device, 60_000)).unwrap();
    assert_eq!(first.status, CommandStatus::Pending);
    assert_eq!(first.created_ms, 1_000_000);
    assert_eq!(first.expires_ms, 1_060_000);
    assert_eq!(first.source, "task:remind");
    assert_eq!(first.result, None);
    let (_, event) = events.try_recv().unwrap().unwrap();
    assert_eq!(
        event,
        LiveEvent::DeviceCommand {
            device: device.clone(),
            id: first.id.clone()
        }
    );
    clock.advance(1);
    let second = state.queue_device_command(&sms(&device, 60_000)).unwrap();

    let pending = state
        .device_commands(&device, Some(CommandStatus::Pending))
        .unwrap();
    assert_eq!(
        pending.iter().map(|c| &c.id).collect::<Vec<_>>(),
        [&first.id, &second.id]
    );

    clock.advance(10);
    let delivered = state
        .report_device_command(&device, &first.id, CommandStatus::Delivered, None)
        .unwrap();
    assert_eq!(delivered.status, CommandStatus::Delivered);
    assert_eq!(delivered.updated_ms, Some(1_000_011));
    let done = state
        .report_device_command(
            &device,
            &first.id,
            CommandStatus::Done,
            Some(r#"{"device_ref":"sms:42"}"#),
        )
        .unwrap();
    assert_eq!(done.status, CommandStatus::Done);
    assert_eq!(done.result.as_deref(), Some(r#"{"device_ref":"sms:42"}"#));
    let failed = state
        .report_device_command(
            &device,
            &second.id,
            CommandStatus::Failed,
            Some(r#""no signal""#),
        )
        .unwrap();
    assert_eq!(failed.status, CommandStatus::Failed);

    assert!(matches!(
        state.report_device_command(&device, &first.id, CommandStatus::Failed, None),
        Err(Error::CommandClosed { status: "done", .. })
    ));
    assert!(matches!(
        state.report_device_command(&device, &second.id, CommandStatus::Pending, None),
        Err(Error::InvalidCommandReport(_))
    ));
    assert_eq!(state.device_commands(&device, None).unwrap().len(), 2);
    assert!(
        state
            .device_commands(&device, Some(CommandStatus::Pending))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_unfinished_command_past_its_expiry_reads_as_expired() {
    let dir = TempDir::new().unwrap();
    let (state, clock, device) = open(&dir);
    let c = state.queue_device_command(&sms(&device, 1_000)).unwrap();
    let d = state.queue_device_command(&sms(&device, 1_000)).unwrap();
    state
        .report_device_command(&device, &d.id, CommandStatus::Delivered, None)
        .unwrap();
    clock.advance(999);
    assert_eq!(
        state.device_command(&device, &c.id).unwrap().status,
        CommandStatus::Pending
    );
    clock.advance(1);
    assert_eq!(
        state.device_command(&device, &c.id).unwrap().status,
        CommandStatus::Expired
    );
    let expired = state
        .device_commands(&device, Some(CommandStatus::Expired))
        .unwrap();
    assert_eq!(expired.len(), 2);
    assert!(matches!(
        state.report_device_command(&device, &c.id, CommandStatus::Done, Some("{}")),
        Err(Error::CommandClosed {
            status: "expired",
            ..
        })
    ));
}

#[test]
fn commands_belong_to_their_device_and_go_with_it() {
    let dir = TempDir::new().unwrap();
    let (state, clock, device) = open(&dir);
    let c = state.queue_device_command(&sms(&device, 1_000)).unwrap();
    assert!(matches!(
        state.queue_device_command(&sms("01NOSUCHDEVICE", 1_000)),
        Err(Error::UnknownDevice(_))
    ));
    assert!(matches!(
        state.device_command("01NOSUCHDEVICE", &c.id),
        Err(Error::UnknownCommand(_))
    ));
    assert!(matches!(
        state.device_commands("01NOSUCHDEVICE", None),
        Err(Error::UnknownDevice(_))
    ));

    // A finished command is kept for the retention period, then removed by the next queueing.
    state
        .report_device_command(&device, &c.id, CommandStatus::Done, None)
        .unwrap();
    clock.advance(COMMAND_RETENTION_MS);
    state.queue_device_command(&sms(&device, 1_000)).unwrap();
    assert!(matches!(
        state.device_command(&device, &c.id),
        Err(Error::UnknownCommand(_))
    ));

    state.revoke_device(&device).unwrap();
    let conn = Connection::open(dir.path().join("state.db")).unwrap();
    let left: i64 = conn
        .query_row("SELECT COUNT(*) FROM device_commands", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn a_version_9_database_gains_the_command_queue() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    drop(open(&dir));
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE device_commands; PRAGMA user_version = 9;")
            .unwrap();
    }
    let state = State::open(&path, None).unwrap();
    let device = state.list_devices().unwrap().remove(0).id;
    state.queue_device_command(&sms(&device, 1_000)).unwrap();
    drop(state);
    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 13);
}
