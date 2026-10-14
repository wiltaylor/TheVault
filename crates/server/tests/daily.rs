//! S1 with S3 checks: the built-in daily note type and the note for a day.

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use testkit::TestServer;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Self {
        Api {
            server: TestServer::start(),
            client: Client::new(),
        }
    }

    fn send(&self, method: &str, path: &str) -> Response {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
            .send()
            .unwrap()
    }

    fn put_type(&self, wcl: &str) -> Response {
        self.client
            .put(self.server.url("/api/types/dailynote"))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn type_file(&self) -> String {
        std::fs::read_to_string(self.server.vault_path().join("types/dailynote.wcl")).unwrap()
    }
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

#[test]
fn every_vault_has_the_daily_note_type() {
    let api = Api::start();
    let ty = expect(api.send("GET", "/api/types/dailynote"), StatusCode::OK);
    assert!(api.type_file().contains("field date"), "{ty}");
}

#[test]
fn opening_a_day_creates_its_note_once_and_finds_it_after() {
    let api = Api::start();
    expect(
        api.send("GET", "/api/daily/2026-10-13"),
        StatusCode::NOT_FOUND,
    );

    let made = expect(
        api.send("POST", "/api/daily/2026-10-13"),
        StatusCode::CREATED,
    );
    assert_eq!(made["created"], true);
    assert_eq!(made["type"], "dailynote");
    assert_eq!(made["note"]["fields"]["title"], "Tuesday 13 October 2026");
    assert_eq!(made["note"]["fields"]["date"], "2026-10-13");
    let id = made["id"].as_str().unwrap().to_owned();
    let file = api
        .server
        .vault_path()
        .join(format!("notes/dailynote/{id}.wcl"));
    assert!(file.is_file(), "{}", file.display());

    let again = expect(api.send("POST", "/api/daily/2026-10-13"), StatusCode::OK);
    assert_eq!(again["id"], id.as_str());
    assert_eq!(again["created"], false);
    let read = expect(api.send("GET", "/api/daily/2026-10-13"), StatusCode::OK);
    assert_eq!(read["id"], id.as_str());

    // Another day is another note.
    let other = expect(
        api.send("POST", "/api/daily/2026-10-14"),
        StatusCode::CREATED,
    );
    assert_ne!(other["id"], id.as_str());
}

#[test]
fn opening_one_day_from_many_clients_at_once_makes_one_note() {
    let api = std::sync::Arc::new(Api::start());
    let ids: Vec<String> = (0..8)
        .map(|_| {
            let api = api.clone();
            std::thread::spawn(move || {
                let res = api.send("POST", "/api/daily/2026-11-01");
                let status = res.status();
                assert!(status == StatusCode::CREATED || status == StatusCode::OK);
                let body: Value = res.json().unwrap();
                body["id"].as_str().unwrap().to_owned()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    assert!(ids.iter().all(|i| i == &ids[0]), "{ids:?}");
    let dir = api.server.vault_path().join("notes/dailynote");
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
}

#[test]
fn a_day_that_is_not_a_real_date_is_a_400() {
    let api = Api::start();
    for bad in ["2026-02-30", "today", "2026-1-1"] {
        expect(
            api.send("POST", &format!("/api/daily/{bad}")),
            StatusCode::BAD_REQUEST,
        );
        expect(
            api.send("GET", &format!("/api/daily/{bad}")),
            StatusCode::BAD_REQUEST,
        );
    }
}

#[test]
fn the_daily_note_type_cannot_be_deleted() {
    let api = Api::start();
    let body = expect(
        api.send("DELETE", "/api/types/dailynote"),
        StatusCode::CONFLICT,
    );
    assert_eq!(body["error"], "built_in_type");
    assert!(api.type_file().contains("field date"));
}

#[test]
fn the_daily_note_type_can_be_customised_but_keeps_one_required_date() {
    let api = Api::start();
    let base = api.type_file();

    let custom = base.replace(
        "field body {",
        "field mood {\n  kind  = \"string\"\n  label = \"Mood\"\n}\n\nfield body {",
    );
    expect(api.put_type(&custom), StatusCode::OK);
    assert!(api.type_file().contains("field mood"));
    let customised = api.type_file();

    let without_date = customised.replace(
        "field date {\n  kind     = \"date\"\n  label    = \"Date\"\n  required = true\n}\n",
        "",
    );
    assert!(!without_date.contains("field date"), "{without_date}");
    let res = expect(api.put_type(&without_date), StatusCode::BAD_REQUEST);
    assert_eq!(res["error"], "invalid_type");

    let as_text = customised.replace("kind     = \"date\"", "kind     = \"string\"");
    assert_ne!(as_text, customised);
    expect(api.put_type(&as_text), StatusCode::BAD_REQUEST);

    let optional = customised.replace("  required = true\n", "");
    assert_ne!(optional, customised);
    expect(api.put_type(&optional), StatusCode::BAD_REQUEST);

    assert_eq!(api.type_file(), customised);
}
