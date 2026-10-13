//! S1: notes pinned to the sidebar. Pins live in `state.db`, keep their order, leave out notes in
//! the trash, and need `write` to change.

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label = "Task"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Self {
        let server = TestServer::start_seeded(|vault| {
            std::fs::create_dir_all(vault.join("types")).unwrap();
            std::fs::write(vault.join("types/task.wcl"), TASK_TYPE).unwrap();
        });
        Api {
            server,
            client: Client::new(),
        }
    }

    fn send(&self, method: &str, path: &str, token: &str) -> Response {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .unwrap()
    }

    fn admin(&self, method: &str, path: &str) -> Response {
        self.send(method, path, self.server.admin_token())
    }

    fn create(&self, title: &str) -> (String, String) {
        let res = self
            .client
            .post(self.server.url("/api/notes?type=task"))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
            .json(&json!({ "note": { "fields": { "title": title }, "blocks": [] } }))
            .send()
            .unwrap();
        let body = expect(res, StatusCode::CREATED);
        (
            body["id"].as_str().unwrap().to_owned(),
            body["version"].as_str().unwrap().to_owned(),
        )
    }

    fn pins(&self) -> Vec<(String, String)> {
        let body = expect(self.admin("GET", "/api/pins"), StatusCode::OK);
        body["pins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                assert_eq!(p["type"], "task");
                (
                    p["id"].as_str().unwrap().to_owned(),
                    p["title"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn token(&self, scope: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/tokens"))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
            .json(&json!({ "name": format!("t-{scope}"), "scope": scope }))
            .send()
            .unwrap();
        expect(res, StatusCode::CREATED)["secret"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

#[test]
fn pinned_notes_come_back_in_the_order_they_were_pinned_with_their_titles() {
    let api = Api::start();
    let (a, _) = api.create("Alpha");
    let (b, _) = api.create("Bravo");
    assert!(api.pins().is_empty());

    let first = expect(api.admin("PUT", &format!("/api/pins/{b}")), StatusCode::OK);
    assert_eq!(first["added"], true);
    expect(api.admin("PUT", &format!("/api/pins/{a}")), StatusCode::OK);
    let again = expect(api.admin("PUT", &format!("/api/pins/{b}")), StatusCode::OK);
    assert_eq!(again["added"], false);

    assert_eq!(
        api.pins(),
        [
            (b.clone(), "Bravo".to_owned()),
            (a.clone(), "Alpha".to_owned())
        ]
    );

    expect(
        api.admin("DELETE", &format!("/api/pins/{b}")),
        StatusCode::NO_CONTENT,
    );
    assert_eq!(api.pins(), [(a, "Alpha".to_owned())]);
    expect(
        api.admin("DELETE", &format!("/api/pins/{b}")),
        StatusCode::NOT_FOUND,
    );
}

#[test]
fn pinning_a_note_that_does_not_exist_is_a_404() {
    let api = Api::start();
    expect(
        api.admin("PUT", "/api/pins/01ZZZZZZZZZZZZZZZZZZZZZZZZ"),
        StatusCode::NOT_FOUND,
    );
    assert!(api.pins().is_empty());
}

#[test]
fn a_pinned_note_in_the_trash_is_left_out_until_it_is_restored() {
    let api = Api::start();
    let (id, version) = api.create("Binned");
    expect(api.admin("PUT", &format!("/api/pins/{id}")), StatusCode::OK);

    expect(
        api.admin("DELETE", &format!("/api/notes/{id}?base_version={version}")),
        StatusCode::OK,
    );
    assert!(api.pins().is_empty());

    expect(
        api.admin("POST", &format!("/api/trash/{id}/restore")),
        StatusCode::OK,
    );
    assert_eq!(api.pins(), [(id, "Binned".to_owned())]);
}

#[test]
fn reading_pins_needs_read_and_changing_them_needs_write() {
    let api = Api::start();
    let (id, _) = api.create("Scoped");
    let read = api.token("read");
    let write = api.token("write");

    expect(api.send("GET", "/api/pins", &read), StatusCode::OK);
    expect(
        api.send("PUT", &format!("/api/pins/{id}"), &read),
        StatusCode::FORBIDDEN,
    );
    expect(
        api.send("PUT", &format!("/api/pins/{id}"), &write),
        StatusCode::OK,
    );
    expect(
        api.send("DELETE", &format!("/api/pins/{id}"), &read),
        StatusCode::FORBIDDEN,
    );
    expect(
        api.send("DELETE", &format!("/api/pins/{id}"), &write),
        StatusCode::NO_CONTENT,
    );
}
