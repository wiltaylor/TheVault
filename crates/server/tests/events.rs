//! S1: the live event stream at `/api/events` (D4). An authenticated client receives each
//! notification event and `note.changed` after a write, in order; an unauthenticated one gets a
//! 401; and an open stream does not keep the server from shutting down.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::TestServer;

const TASK: &str = r#"label = "Task"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

/// How long to wait for an event that should come.
const WAIT: Duration = Duration::from_secs(20);

fn seed(vault: &std::path::Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/task.wcl"), TASK).unwrap();
}

/// One event off the stream: `(event, id, data)`.
#[derive(Debug)]
struct Frame {
    event: String,
    id: Option<u64>,
    data: Value,
}

/// The frames of an open stream, read on a thread of their own. Comments are skipped.
struct Stream {
    frames: Receiver<Frame>,
}

impl Stream {
    fn open(res: Response) -> Stream {
        assert_eq!(res.status().as_u16(), 200);
        assert!(
            res.headers()[CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream"),
            "{:?}",
            res.headers()
        );
        let (tx, frames) = mpsc::channel();
        thread::spawn(move || {
            let mut event = String::new();
            let mut id = None;
            let mut data = String::new();
            for line in BufReader::new(res).lines() {
                let Ok(line) = line else { return };
                if line.is_empty() {
                    if !event.is_empty() || !data.is_empty() {
                        let frame = Frame {
                            event: std::mem::take(&mut event),
                            id: id.take(),
                            data: serde_json::from_str(&std::mem::take(&mut data))
                                .unwrap_or(Value::Null),
                        };
                        if tx.send(frame).is_err() {
                            return;
                        }
                    }
                } else if let Some(v) = line.strip_prefix("event:") {
                    event = v.trim().to_owned();
                } else if let Some(v) = line.strip_prefix("id:") {
                    id = v.trim().parse().ok();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push_str(v.trim_start());
                }
            }
        });
        Stream { frames }
    }

    fn next(&self) -> Frame {
        self.frames
            .recv_timeout(WAIT)
            .expect("an event within the wait")
    }

    /// Nothing more arrives within a short wait.
    fn assert_quiet(&self) {
        if let Ok(f) = self.frames.recv_timeout(Duration::from_millis(500)) {
            panic!("unexpected event {f:?}");
        }
    }
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        Api {
            server: TestServer::start_seeded(seed),
            client: Client::new(),
        }
    }

    fn req(&self, method: &str, path: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>, status: u16) -> Value {
        let mut req = self.req(method, path);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().unwrap();
        let got = res.status().as_u16();
        let text = res.text().unwrap();
        assert_eq!(got, status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    fn events(&self) -> Stream {
        Stream::open(self.req("GET", "/api/events").send().unwrap())
    }
}

#[test]
fn an_unauthenticated_client_gets_401() {
    let api = Api::start();
    let res = api
        .client
        .get(api.server.url("/api/events"))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);
    let res = api
        .client
        .get(api.server.url("/api/events"))
        .header(AUTHORIZATION, "Bearer not-a-token")
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);
}

#[test]
fn a_browser_session_can_subscribe() {
    let api = Api::start();
    let res = api
        .client
        .post(api.server.url("/api/login"))
        .json(&json!({ "password": testkit::PASSWORD }))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let cookie = res.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let stream = Stream::open(
        api.client
            .get(api.server.url("/api/events"))
            .header("cookie", cookie)
            .send()
            .unwrap(),
    );
    let n = api.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "hello" })),
        201,
    );
    let f = stream.next();
    assert_eq!(f.event, "notification.created");
    assert_eq!(f.data["id"], n["id"]);
}

#[test]
fn every_notification_event_and_note_change_arrives_in_order() {
    let api = Api::start();
    let stream = api.events();
    // A second subscriber sees the same events.
    let other = api.events();

    let note = api.call(
        "POST",
        "/api/notes?type=task",
        Some(json!({ "note": { "fields": { "title": "Draft" } } })),
        201,
    );
    let id = note["id"].as_str().unwrap().to_owned();
    let a = api.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "first", "note": id })),
        201,
    )["id"]
        .clone();
    let b = api.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "second" })),
        201,
    )["id"]
        .clone();
    api.call(
        "POST",
        &format!("/api/notifications/{}/read", a.as_str().unwrap()),
        None,
        200,
    );
    api.call(
        "DELETE",
        &format!("/api/notifications/{}", b.as_str().unwrap()),
        None,
        200,
    );
    let updated = api.call(
        "PUT",
        &format!("/api/notes/{id}"),
        Some(json!({
            "base_version": note["version"],
            "note": { "fields": { "title": "Final" } },
        })),
        200,
    );
    let c = api.call(
        "POST",
        "/api/notifications",
        Some(json!({ "title": "third" })),
        201,
    )["id"]
        .clone();
    api.call("POST", "/api/notifications/read-all", None, 200);
    api.call("DELETE", "/api/notifications", None, 200);
    api.call(
        "DELETE",
        &format!(
            "/api/notes/{id}?base_version={}",
            updated["version"].as_str().unwrap()
        ),
        None,
        200,
    );

    let expected = [
        (
            "note.changed",
            json!({ "id": id, "type": "task", "version": note["version"], "deleted": false }),
        ),
        ("notification.created", json!({ "id": a, "unread": 1 })),
        ("notification.created", json!({ "id": b, "unread": 2 })),
        ("notification.read", json!({ "id": a, "unread": 1 })),
        ("notification.dismissed", json!({ "id": b, "unread": 0 })),
        (
            "note.changed",
            json!({ "id": id, "type": "task", "version": updated["version"], "deleted": false }),
        ),
        ("notification.created", json!({ "id": c, "unread": 1 })),
        ("notification.read", json!({ "id": null, "unread": 0 })),
        ("notification.cleared", json!({ "cleared": 2, "unread": 0 })),
    ];
    for s in [&stream, &other] {
        let mut last = 0;
        for (event, data) in &expected {
            let f = s.next();
            assert_eq!((f.event.as_str(), &f.data), (*event, data));
            let seq = f.id.expect("every event has an id");
            assert!(seq > last, "ids go up: {seq} after {last}");
            last = seq;
        }
        let f = s.next();
        assert_eq!(f.event, "note.changed");
        assert_eq!(f.data["id"], json!(id));
        assert_eq!(f.data["deleted"], json!(true));
        s.assert_quiet();
    }
}

#[test]
fn an_open_stream_does_not_stop_the_server_shutting_down() {
    let vault = tempfile::tempdir().unwrap();
    let token = testkit::seed_credentials(vault.path());
    let mut child = Command::new(testkit::server_binary())
        .arg("--vault")
        .arg(vault.path())
        .args(["--listen", "127.0.0.1:0"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let listen: Value = serde_json::from_str(&line).unwrap();
    let url = format!("http://{}/api/events", listen["listen"].as_str().unwrap());
    let _stream = Stream::open(
        Client::new()
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .unwrap(),
    );

    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the server did not stop with an event stream open");
        }
        thread::sleep(Duration::from_millis(50));
    }
}
