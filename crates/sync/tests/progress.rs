//! A first sync's progress, as the desktop setup page and the sync popover show it (ticket 144;
//! D19, D20, D29).
//!
//! A testkit home server holds more notes than one feed page and a few megabytes of assets. A
//! desktop device syncs from it for the first time through a proxy that slows what the server
//! sends, so the round lasts a couple of seconds, while a second thread reads the round's
//! progress every few milliseconds, as the app's status poll does.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use embedded::{Config, Embedded};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use sync::{AssetPolicy, Progress, Remote, Stage};
use testkit::{PASSWORD, TestServer};

const MEMO: &str = r#"label = "Memo"

field pic {
  kind = "asset"
}

template default {
  tab "Main" {
    slot "title"
    slot "pic"
  }
}
"#;

/// More than one feed page of 500.
const NOTES: usize = 620;

/// Bytes a second the proxy lets through from the server to the device.
const RATE: u64 = 2 * 1024 * 1024;

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

/// A TCP proxy in front of the home server that passes what the device sends as it comes, and
/// what the server sends at no more than [`RATE`] bytes a second per connection.
fn throttling_proxy(upstream: SocketAddr) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(down) = conn else { return };
            let Ok(up) = TcpStream::connect(upstream) else {
                continue;
            };
            let (down2, up2) = (down.try_clone().unwrap(), up.try_clone().unwrap());
            std::thread::spawn(move || pump(down, up, None));
            std::thread::spawn(move || pump(up2, down2, Some(RATE)));
        }
    });
    addr
}

fn pump(mut from: TcpStream, mut to: TcpStream, rate: Option<u64>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).is_err() {
            break;
        }
        if let Some(rate) = rate {
            std::thread::sleep(Duration::from_secs_f64(n as f64 / rate as f64));
        }
    }
    let _ = to.shutdown(Shutdown::Write);
}

fn str(v: &Value) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("not a string: {v}"))
        .to_owned()
}

fn admin(server: &TestServer, rb: reqwest::blocking::RequestBuilder, status: u16) -> Value {
    let res = rb.bearer_auth(server.admin_token()).send().unwrap();
    assert_eq!(res.status().as_u16(), status);
    res.json().unwrap()
}

#[test]
fn a_first_sync_reports_monotonic_progress_with_the_right_totals_and_ends_done() {
    let server = TestServer::start_seeded(seed);
    let http = Client::new();

    // Assets of a few sizes, 7.5 MB in all, each named by a note.
    let sizes: [usize; 4] = [3 * 1024 * 1024, 2 * 1024 * 1024, 1536 * 1024, 1024 * 1024];
    let mut pics = Vec::new();
    for (i, size) in sizes.iter().enumerate() {
        let bytes: Vec<u8> = (0..*size).map(|k| (k % 251) as u8).collect();
        let out = admin(
            &server,
            http.post(server.url("/api/assets"))
                .query(&[("filename", format!("pic-{i}.bin"))])
                .body(bytes),
            201,
        );
        pics.push(str(&out["id"]));
    }
    for i in 0..NOTES {
        let mut fields = json!({ "title": format!("Note {i}") });
        if let Some(pic) = pics.get(i) {
            fields["pic"] = json!(pic);
        }
        admin(
            &server,
            http.post(server.url("/api/notes?type=memo"))
                .json(&json!({ "note": { "fields": fields } })),
            201,
        );
    }
    // Every feed entry, notes and definition files: what the pull reads.
    let feed = admin(
        &server,
        http.get(server.url("/api/sync/changes?limit=5000")),
        200,
    );
    let entries = feed["changes"].as_array().unwrap().len() as u64;
    assert!(entries > NOTES as u64, "{entries}");

    let paired = admin(
        &server,
        http.post(server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": "Laptop", "platform": "linux" })),
        201,
    );
    let proxy = throttling_proxy(server.addr());
    let vault = tempfile::Builder::new()
        .prefix("thevault-device-")
        .tempdir()
        .unwrap();
    let device = Embedded::open(Config {
        vault: vault.path().to_owned(),
        web: None,
        secret_key: None,
    })
    .unwrap();
    let engine = device
        .sync_engine(
            Remote {
                url: format!("http://{proxy}"),
                token: str(&paired["token"]),
                device_id: str(&paired["device_id"]),
                device_name: "Laptop".to_owned(),
            },
            AssetPolicy::desktop(),
        )
        .unwrap();
    assert_eq!(device.sync_progress().snapshot().stage, Stage::Idle);

    // Read the progress as the status poll does, from another thread, until the round ends.
    let tracker = device.sync_progress().clone();
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut seen: Vec<Progress> = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                seen.push(tracker.snapshot());
                std::thread::sleep(Duration::from_millis(5));
            }
            seen
        })
    };
    let report = engine.sync().unwrap();
    stop.store(true, Ordering::SeqCst);
    let seen = watcher.join().unwrap();
    assert_eq!(report.pulled as u64, entries, "{report:?}");
    assert_eq!(report.assets_fetched, sizes.len(), "{report:?}");

    // Every count and every total only grows, and no count passes its total.
    let fields = |p: &Progress| {
        [
            p.notes_done,
            p.notes_total,
            p.files_done,
            p.files_total,
            p.bytes_done,
            p.bytes_total,
        ]
    };
    for pair in seen.windows(2) {
        let (a, b) = (fields(&pair[0]), fields(&pair[1]));
        assert!(
            a.iter().zip(&b).all(|(x, y)| x <= y),
            "went back: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    for p in &seen {
        assert!(p.notes_done <= p.notes_total, "{p:?}");
        assert!(p.files_done <= p.files_total, "{p:?}");
        assert!(p.bytes_done <= p.bytes_total, "{p:?}");
    }

    // The round went through its stages in order.
    let mut stages: Vec<Stage> = seen.iter().map(|p| p.stage).collect();
    stages.dedup();
    let order = |s: &Stage| match s {
        Stage::Idle => 0,
        Stage::Handshake => 1,
        Stage::Pushing => 2,
        Stage::Pulling => 3,
        Stage::Fetching => 4,
        Stage::Done | Stage::Failed => 5,
    };
    assert!(
        stages.windows(2).all(|w| order(&w[0]) < order(&w[1])),
        "{stages:?}"
    );
    assert!(stages.contains(&Stage::Pulling), "{stages:?}");
    assert!(stages.contains(&Stage::Fetching), "{stages:?}");

    // The totals were right while the round ran, not only at its end: the files and bytes from
    // the start of the pull, and the notes from the first feed page.
    let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();
    let pulling = seen
        .iter()
        .find(|p| p.stage == Stage::Pulling && p.notes_total > 0)
        .expect("a snapshot while pulling");
    assert_eq!(pulling.notes_total, entries, "{pulling:?}");
    assert_eq!(pulling.files_total, sizes.len() as u64, "{pulling:?}");
    assert_eq!(pulling.bytes_total, total_bytes, "{pulling:?}");
    // Part way through the pull there was an estimate of the time left, the files' bytes guessed
    // from the feed's own rate.
    assert!(
        seen.iter()
            .any(|p| p.stage == Stage::Pulling && p.eta_secs.is_some_and(|s| s > 0)),
        "no estimate while pulling"
    );
    // Part way through the files, bytes had arrived and there was an estimate of the time left.
    assert!(
        seen.iter().any(|p| p.stage == Stage::Fetching
            && p.bytes_done > 0
            && p.bytes_done < total_bytes
            && p.eta_secs.is_some_and(|s| s > 0)),
        "no estimate while fetching: {:?}",
        seen.iter()
            .filter(|p| p.stage == Stage::Fetching)
            .map(|p| (p.elapsed_ms, p.bytes_done, p.eta_secs))
            .collect::<Vec<_>>()
    );

    // The final report says done, with every count at its total and the bytes the asset sizes.
    let last = device.sync_progress().snapshot();
    assert_eq!(last.stage, Stage::Done, "{last:?}");
    assert_eq!((last.notes_done, last.notes_total), (entries, entries));
    assert_eq!(
        (last.files_done, last.files_total),
        (sizes.len() as u64, sizes.len() as u64)
    );
    assert_eq!(
        (last.bytes_done, last.bytes_total),
        (total_bytes, total_bytes)
    );
    assert_eq!(last.eta_secs, Some(0));
    assert!(last.is_large());

    // The status the sync popover reads carries it.
    let status = device.sync_status();
    assert_eq!(status["progress"]["stage"], "done", "{status}");
    assert_eq!(status["progress"]["bytes_done"], total_bytes, "{status}");
    assert_eq!(status["progress"]["notes_total"], entries, "{status}");
    assert_eq!(status["progress"]["large"], true, "{status}");

    // A second round has nothing to fetch and starts from zero.
    engine.sync().unwrap();
    let again = device.sync_progress().snapshot();
    assert_eq!(again.stage, Stage::Done);
    assert_eq!(
        (again.notes_total, again.files_total, again.bytes_total),
        (0, 0, 0)
    );
    assert!(!again.is_large());
}
