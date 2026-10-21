//! Asset uploads over a slow, a stalling and a failing network (ticket 141; D19, D29).
//!
//! A device node from `crates/embedded` syncs with a testkit home server through a TCP proxy in
//! this test that can throttle or stall what the device sends. Sync rounds run on the test
//! thread, outside any async runtime, since the engine blocks.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, header};
use embedded::{Config, Embedded};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use sync::{AssetPolicy, Engine, Remote, Timeouts};
use tempfile::TempDir;
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

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

/// What the proxy does with the bytes the device sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Pass,
    /// At most this many bytes a second.
    Throttle(u64),
    /// Stop reading from a connection once it has carried this many bytes.
    StallAfter(u64),
}

/// A TCP proxy in front of the home server, on a port of its own.
struct Proxy {
    addr: SocketAddr,
    mode: Arc<Mutex<Mode>>,
}

impl Proxy {
    fn start(upstream: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mode = Arc::new(Mutex::new(Mode::Pass));
        let shared = mode.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(down) = conn else { return };
                let Ok(up) = TcpStream::connect(upstream) else {
                    continue;
                };
                let (down2, up2) = (down.try_clone().unwrap(), up.try_clone().unwrap());
                std::thread::spawn(move || pump(up2, down2, None));
                let mode = shared.clone();
                std::thread::spawn(move || pump(down, up, Some(mode)));
            }
        });
        Proxy { addr, mode }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn set(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }
}

/// Copy `from` to `to`, as `mode` says when there is one.
fn pump(mut from: TcpStream, mut to: TcpStream, mode: Option<Arc<Mutex<Mode>>>) {
    let mut buf = vec![0u8; 16 * 1024];
    let mut carried: u64 = 0;
    loop {
        let m = mode.as_ref().map_or(Mode::Pass, |m| *m.lock().unwrap());
        let max = match m {
            Mode::Pass => buf.len(),
            Mode::Throttle(rate) => ((rate / 10) as usize).clamp(1, buf.len()),
            Mode::StallAfter(limit) if carried >= limit => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Mode::StallAfter(limit) => ((limit - carried) as usize).min(buf.len()),
        };
        let n = match from.read(&mut buf[..max]) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).is_err() {
            break;
        }
        carried += n as u64;
        if let Mode::Throttle(rate) = m {
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

/// A device on its own local vault, paired with the home server, which it reaches through
/// `url`.
struct Device {
    embedded: Embedded,
    remote: Remote,
    runtime: tokio::runtime::Runtime,
    _vault: TempDir,
}

impl Device {
    fn pair(server: &TestServer, url: &str) -> Device {
        let res = Client::new()
            .post(server.url("/api/devices/pair"))
            .bearer_auth(server.admin_token())
            .json(&json!({ "password": PASSWORD, "name": "Phone", "platform": "android" }))
            .send()
            .unwrap();
        assert_eq!(res.status(), 201);
        let out: Value = res.json().unwrap();
        let vault = tempfile::Builder::new()
            .prefix("thevault-device-")
            .tempdir()
            .unwrap();
        let embedded = Embedded::open(Config {
            vault: vault.path().to_owned(),
            web: None,
            secret_key: None,
        })
        .unwrap();
        Device {
            embedded,
            remote: Remote {
                url: url.to_owned(),
                token: str(&out["token"]),
                device_id: str(&out["device_id"]),
                device_name: "Phone".to_owned(),
            },
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            _vault: vault,
        }
    }

    fn engine(&self, timeouts: Timeouts) -> Engine {
        self.embedded
            .sync_engine(self.remote.clone(), AssetPolicy::android())
            .unwrap()
            .with_timeouts(timeouts)
            .unwrap()
    }

    /// One request to the device's own server, as its webview makes it.
    fn call(&self, method: &str, path: &str, body: Body, status: u16) -> Value {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap();
        let (got, bytes) = self.runtime.block_on(async {
            let res = self.embedded.handle(req).await;
            let got = res.status().as_u16();
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            (got, bytes)
        });
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(got, status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    /// Store `bytes` as an asset on the device; returns its id.
    fn asset(&self, filename: &str, bytes: Vec<u8>) -> String {
        let out = self.call(
            "POST",
            &format!("/api/assets?filename={filename}"),
            Body::from(bytes),
            201,
        );
        str(&out["id"])
    }

    /// Create a memo on the device, naming asset `pic` when given; returns its id.
    fn memo(&self, title: &str, pic: Option<&str>) -> String {
        let mut fields = json!({ "title": title });
        if let Some(pic) = pic {
            fields["pic"] = json!(pic);
        }
        let note = json!({ "note": { "fields": fields } });
        let out = self.call(
            "POST",
            "/api/notes?type=memo",
            Body::from(note.to_string()),
            201,
        );
        str(&out["id"])
    }

    fn status(&self) -> Value {
        self.call("GET", "/api/app/sync", Body::empty(), 200)
    }
}

/// The server's note `id`, or `None` when it has none.
fn server_note(server: &TestServer, id: &str) -> Option<Value> {
    let res = Client::new()
        .get(server.url(&format!("/api/notes/{id}")))
        .bearer_auth(server.admin_token())
        .send()
        .unwrap();
    (res.status() == 200).then(|| res.json().unwrap())
}

fn server_asset(server: &TestServer, id: &str) -> Option<Vec<u8>> {
    let res = Client::new()
        .get(server.url(&format!("/api/assets/{id}")))
        .bearer_auth(server.admin_token())
        .send()
        .unwrap();
    (res.status() == 200).then(|| res.bytes().unwrap().to_vec())
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|k| (k % 251) as u8).collect()
}

#[test]
fn a_slow_upload_that_keeps_moving_completes_however_long_it_takes() {
    let server = TestServer::start_seeded(seed);
    let proxy = Proxy::start(server.addr());
    let phone = Device::pair(&server, &proxy.url());
    phone.engine(Timeouts::default()).sync().unwrap();

    // 9 MiB at 200 KiB a second: about 45 s, longer than any flat timeout the old client had,
    // and the stall timeout is the default 60 s.
    let bytes = pattern(9 * 1024 * 1024);
    let pic = phone.asset("video.mp4", bytes.clone());
    let note = phone.memo("With a video", Some(&pic));
    proxy.set(Mode::Throttle(200 * 1024));
    let started = Instant::now();
    let report = phone.engine(Timeouts::default()).sync().unwrap();
    let took = started.elapsed();

    assert!(
        took >= Duration::from_secs(40),
        "the upload was not slowed: {took:?}"
    );
    assert_eq!(report.assets_sent, 1, "{report:?}");
    assert_eq!(report.assets_failed, 0, "{report:?}");
    assert!(report.stuck.is_empty(), "{report:?}");
    assert_eq!(report.pushed, std::slice::from_ref(&note), "{report:?}");
    assert_eq!(report.problem(), None);
    assert_eq!(server_asset(&server, &pic).unwrap(), bytes);
    assert_eq!(
        server_note(&server, &note).unwrap()["note"]["fields"]["pic"],
        pic
    );
}

#[test]
fn a_stalled_upload_fails_alone_and_the_notes_that_do_not_need_it_sync() {
    let server = TestServer::start_seeded(seed);
    let proxy = Proxy::start(server.addr());
    let phone = Device::pair(&server, &proxy.url());
    let quick = Timeouts {
        stall: Duration::from_secs(2),
        asset_retry: Duration::from_secs(60 * 60),
        ..Timeouts::default()
    };
    phone.engine(quick).sync().unwrap();

    let small = phone.asset("small.png", pattern(10));
    let big_bytes = pattern(32 * 1024 * 1024);
    let big = phone.asset("big.mp4", big_bytes.clone());
    let with_small = phone.memo("With the small one", Some(&small));
    let with_big = phone.memo("With the big one", Some(&big));
    let plain = phone.memo("Plain", None);

    // Every connection stalls after 1 MiB: the big asset never gets through.
    proxy.set(Mode::StallAfter(1024 * 1024));
    let started = Instant::now();
    let report = phone.engine(quick).sync().unwrap();
    let took = started.elapsed();

    assert!(took >= Duration::from_secs(2), "{took:?}");
    assert!(
        took < Duration::from_secs(30),
        "the stall was not caught: {took:?}"
    );
    assert_eq!(report.assets_sent, 1, "{report:?}");
    assert_eq!(report.assets_failed, 1, "{report:?}");
    assert_eq!(report.stuck.len(), 1, "{report:?}");
    let stuck = &report.stuck[0];
    assert_eq!(stuck.id, big);
    assert_eq!(stuck.filename, "big.mp4");
    assert_eq!(stuck.size, 32 * 1024 * 1024);
    assert_eq!(stuck.attempts, 1);
    assert!(stuck.error.contains("stalled"), "{}", stuck.error);
    let mut pushed = report.pushed.clone();
    pushed.sort();
    let mut want = vec![with_small.clone(), plain.clone()];
    want.sort();
    assert_eq!(pushed, want, "{report:?}");
    assert_eq!(report.held.len(), 1, "{report:?}");
    assert_eq!(report.held[0].note_id, with_big);
    assert_eq!(report.held[0].asset_id.as_deref(), Some(big.as_str()));
    let problem = report.problem().unwrap();
    assert!(problem.contains("big.mp4 (32.0 MB)"), "{problem}");
    assert!(problem.contains("1 note waits"), "{problem}");

    assert!(server_note(&server, &with_small).is_some());
    assert!(server_note(&server, &plain).is_some());
    assert!(server_note(&server, &with_big).is_none());
    assert_eq!(server_asset(&server, &small).unwrap(), pattern(10));
    assert!(server_asset(&server, &big).is_none());

    // The device's sync status names the stuck asset and what waits for it.
    let status = phone.status();
    assert_eq!(status["queued"], 1, "{status}");
    assert_eq!(status["held"], 1, "{status}");
    assert_eq!(status["assets_waiting"], 1, "{status}");
    let listed = &status["stuck_assets"][0];
    assert_eq!(listed["filename"], "big.mp4", "{status}");
    assert_eq!(listed["size"], 32 * 1024 * 1024, "{status}");
    assert!(
        listed["error"].as_str().unwrap().contains("stalled"),
        "{status}"
    );

    // The network recovers. The next round waits out the asset's retry time, so the note
    // still waits.
    proxy.set(Mode::Pass);
    let report = phone.engine(quick).sync().unwrap();
    assert_eq!(report.assets_sent, 0, "{report:?}");
    assert_eq!(report.assets_failed, 0, "{report:?}");
    assert_eq!(report.stuck.len(), 1, "{report:?}");
    assert_eq!(report.held.len(), 1, "{report:?}");

    // Once it is due ("Sync now" makes it due at once), it uploads and the held note follows.
    phone.embedded.queue().retry_assets_now().unwrap();
    let report = phone.engine(quick).sync().unwrap();
    assert_eq!(report.assets_sent, 1, "{report:?}");
    assert!(report.stuck.is_empty(), "{report:?}");
    assert!(report.held.is_empty(), "{report:?}");
    assert_eq!(report.pushed, std::slice::from_ref(&with_big), "{report:?}");
    assert_eq!(report.problem(), None);
    assert_eq!(server_asset(&server, &big).unwrap(), big_bytes);
    assert!(server_note(&server, &with_big).is_some());
    let status = phone.status();
    assert_eq!(status["queued"], 0, "{status}");
    assert_eq!(status["assets_waiting"], 0, "{status}");
    assert_eq!(status["stuck_assets"], json!([]), "{status}");
}

#[test]
fn an_asset_the_server_refuses_does_not_stop_the_round() {
    let server = TestServer::start_seeded(seed);
    let phone = Device::pair(&server, &server.base_url());
    phone.engine(Timeouts::default()).sync().unwrap();

    // The server already holds a different file under this id, so it refuses the upload.
    let pic = phone.asset("clash.png", pattern(100));
    let res = Client::new()
        .put(server.url(&format!("/api/assets/{pic}")))
        .query(&[("filename", "other.png")])
        .bearer_auth(server.admin_token())
        .body(vec![9u8; 5])
        .send()
        .unwrap();
    assert_eq!(res.status(), 201, "{}", res.text().unwrap());
    let plain = phone.memo("Plain", None);
    let with_pic = phone.memo("With the clash", Some(&pic));

    let report = phone.engine(Timeouts::default()).sync().unwrap();
    assert_eq!(report.pushed, std::slice::from_ref(&plain), "{report:?}");
    assert_eq!(report.assets_failed, 1, "{report:?}");
    assert_eq!(report.held.len(), 1, "{report:?}");
    assert!(report.held[0].reason.contains("409"), "{report:?}");
    // The server lists an asset under the id, so the device takes it as there and stops
    // retrying; the held note goes with the next round.
    assert!(report.stuck.is_empty(), "{report:?}");
    let report = phone.engine(Timeouts::default()).sync().unwrap();
    assert_eq!(report.pushed, [with_pic], "{report:?}");
    assert!(server_note(&server, &plain).is_some());
}
