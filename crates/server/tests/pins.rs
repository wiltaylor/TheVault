//! S1: the sidebar's pinned tree. Pins live in `state.db`, keep their order, nest under folders
//! and notes, leave out notes in the trash with their children shown in place, and need `write`
//! to change.

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

    fn send_json(&self, method: &str, path: &str, token: &str, body: Value) -> Response {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .json(&body)
            .send()
            .unwrap()
    }

    fn admin_json(&self, method: &str, path: &str, body: Value) -> Response {
        self.send_json(method, path, self.server.admin_token(), body)
    }

    fn folder(&self, name: &str, parent: Option<&str>) -> String {
        let res = self.admin_json(
            "POST",
            "/api/pins/folders",
            json!({ "name": name, "parent": parent }),
        );
        let body = expect(res, StatusCode::CREATED);
        assert_eq!(body["kind"], "folder");
        assert_eq!(body["name"], name);
        assert_eq!(body["children"], json!([]));
        body["id"].as_str().unwrap().to_owned()
    }

    fn mv(&self, id: &str, parent: Option<&str>, index: usize) -> Response {
        self.admin_json(
            "POST",
            "/api/pins/move",
            json!({ "id": id, "parent": parent, "index": index }),
        )
    }

    fn pin(&self, id: &str) {
        expect(
            self.admin("PUT", &format!("/api/pins/{id}")),
            StatusCode::OK,
        );
    }

    /// The tree as `label(child child)` text: notes by title, folders as `[name]`.
    fn shape(&self) -> String {
        fn render(items: &Value) -> String {
            items
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    let label = match p["kind"].as_str().unwrap() {
                        "note" => {
                            assert_eq!(p["type"], "task");
                            p["title"].as_str().unwrap().to_owned()
                        }
                        "folder" => format!("[{}]", p["name"].as_str().unwrap()),
                        other => panic!("unknown kind {other}"),
                    };
                    let kids = &p["children"];
                    if kids.as_array().unwrap().is_empty() {
                        label
                    } else {
                        format!("{label}({})", render(kids))
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        render(&expect(self.admin("GET", "/api/pins"), StatusCode::OK)["pins"])
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
                assert_eq!(p["kind"], "note");
                assert_eq!(p["type"], "task");
                assert_eq!(p["children"], json!([]));
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

    let folder = json!({ "name": "F" });
    expect(
        api.send_json("POST", "/api/pins/folders", &read, folder.clone()),
        StatusCode::FORBIDDEN,
    );
    let made = expect(
        api.send_json("POST", "/api/pins/folders", &write, folder),
        StatusCode::CREATED,
    );
    let fid = made["id"].as_str().unwrap();
    let rename = json!({ "name": "G" });
    expect(
        api.send_json(
            "PUT",
            &format!("/api/pins/folders/{fid}"),
            &read,
            rename.clone(),
        ),
        StatusCode::FORBIDDEN,
    );
    expect(
        api.send_json("PUT", &format!("/api/pins/folders/{fid}"), &write, rename),
        StatusCode::OK,
    );
    let mv = json!({ "id": fid, "parent": null, "index": 0 });
    expect(
        api.send_json("POST", "/api/pins/move", &read, mv.clone()),
        StatusCode::FORBIDDEN,
    );
    expect(
        api.send_json("POST", "/api/pins/move", &write, mv),
        StatusCode::OK,
    );
    expect(
        api.send("DELETE", &format!("/api/pins/folders/{fid}"), &read),
        StatusCode::FORBIDDEN,
    );
    expect(
        api.send("DELETE", &format!("/api/pins/folders/{fid}"), &write),
        StatusCode::NO_CONTENT,
    );
    assert_eq!(api.shape(), "");
}

#[test]
fn folders_are_created_renamed_and_deleted_with_their_children_moving_up() {
    let api = Api::start();
    let (a, _) = api.create("Alpha");
    let (b, _) = api.create("Bravo");
    let (c, _) = api.create("Charlie");
    api.pin(&a);
    let work = api.folder("Work", None);
    api.folder("Inner", Some(&work));
    api.pin(&b);
    api.pin(&c);
    expect(api.mv(&b, Some(&work), 0), StatusCode::OK);
    assert_eq!(api.shape(), "Alpha [Work](Bravo [Inner]) Charlie");

    let renamed = expect(
        api.admin_json(
            "PUT",
            &format!("/api/pins/folders/{work}"),
            json!({ "name": "Projects" }),
        ),
        StatusCode::OK,
    );
    assert_eq!(renamed["name"], "Projects");
    assert_eq!(api.shape(), "Alpha [Projects](Bravo [Inner]) Charlie");

    expect(
        api.admin_json("POST", "/api/pins/folders", json!({ "name": "  " })),
        StatusCode::BAD_REQUEST,
    );
    expect(
        api.admin_json(
            "POST",
            "/api/pins/folders",
            json!({ "name": "X", "parent": "nope" }),
        ),
        StatusCode::NOT_FOUND,
    );
    expect(
        api.admin_json("PUT", "/api/pins/folders/nope", json!({ "name": "X" })),
        StatusCode::NOT_FOUND,
    );

    expect(
        api.admin("DELETE", &format!("/api/pins/folders/{work}")),
        StatusCode::NO_CONTENT,
    );
    assert_eq!(api.shape(), "Alpha Bravo [Inner] Charlie");
    expect(
        api.admin("DELETE", &format!("/api/pins/folders/{work}")),
        StatusCode::NOT_FOUND,
    );
    // A note is not a folder.
    expect(
        api.admin("DELETE", &format!("/api/pins/folders/{a}")),
        StatusCode::NOT_FOUND,
    );
}

#[test]
fn moving_nests_and_reorders_entries_and_refuses_a_move_into_itself() {
    let api = Api::start();
    let (a, _) = api.create("Alpha");
    let (b, _) = api.create("Bravo");
    let (c, _) = api.create("Charlie");
    for id in [&a, &b, &c] {
        api.pin(id);
    }
    let f = api.folder("F", None);

    // A note nested under another note.
    let moved = expect(api.mv(&b, Some(&a), 0), StatusCode::OK);
    assert_eq!(moved["parent"], a.as_str());
    assert_eq!(moved["index"], 0);
    assert_eq!(api.shape(), "Alpha(Bravo) Charlie [F]");

    // The index counts without the moved entry and is clamped.
    expect(api.mv(&a, Some(&f), 99), StatusCode::OK);
    assert_eq!(api.shape(), "Charlie [F](Alpha(Bravo))");
    expect(api.mv(&f, None, 0), StatusCode::OK);
    assert_eq!(api.shape(), "[F](Alpha(Bravo)) Charlie");
    expect(api.mv(&c, None, 0), StatusCode::OK);
    assert_eq!(api.shape(), "Charlie [F](Alpha(Bravo))");
    // Without an index, the entry goes last.
    let last = expect(
        api.admin_json("POST", "/api/pins/move", json!({ "id": c, "parent": null })),
        StatusCode::OK,
    );
    assert_eq!(last["index"], 1);
    assert_eq!(api.shape(), "[F](Alpha(Bravo)) Charlie");

    for target in [&f, &a, &b] {
        let res = expect(api.mv(&f, Some(target), 0), StatusCode::CONFLICT);
        assert_eq!(res["error"], "pin_cycle", "{res}");
    }
    expect(api.mv(&a, Some(&b), 0), StatusCode::CONFLICT);
    expect(api.mv(&a, Some("nope"), 0), StatusCode::NOT_FOUND);
    expect(api.mv("nope", None, 0), StatusCode::NOT_FOUND);
    assert_eq!(api.shape(), "[F](Alpha(Bravo)) Charlie");
}

#[test]
fn unpinning_a_note_moves_its_children_up_into_its_place() {
    let api = Api::start();
    let (a, _) = api.create("Alpha");
    let (b, _) = api.create("Bravo");
    let (c, _) = api.create("Charlie");
    let (d, _) = api.create("Delta");
    for id in [&a, &b, &c, &d] {
        api.pin(id);
    }
    expect(api.mv(&c, Some(&b), 0), StatusCode::OK);
    expect(api.mv(&d, Some(&b), 1), StatusCode::OK);
    expect(api.mv(&a, None, 9), StatusCode::OK);
    assert_eq!(api.shape(), "Bravo(Charlie Delta) Alpha");
    expect(
        api.admin("DELETE", &format!("/api/pins/{b}")),
        StatusCode::NO_CONTENT,
    );
    assert_eq!(api.shape(), "Charlie Delta Alpha");
    // Pinning again puts it at the end of the root; pinning a nested note leaves it in place.
    api.pin(&b);
    expect(api.mv(&a, Some(&c), 0), StatusCode::OK);
    api.pin(&a);
    assert_eq!(api.shape(), "Charlie(Alpha) Delta Bravo");
}

#[test]
fn a_trashed_pinned_note_shows_its_children_in_its_place_until_it_is_restored() {
    let api = Api::start();
    let (a, _) = api.create("Alpha");
    let (p, version) = api.create("Parent");
    let (k, _) = api.create("Kid");
    let (z, _) = api.create("Zulu");
    let f = api.folder("F", None);
    for id in [&a, &p, &k, &z] {
        api.pin(id);
    }
    expect(api.mv(&a, Some(&f), 0), StatusCode::OK);
    expect(api.mv(&p, Some(&f), 1), StatusCode::OK);
    expect(api.mv(&z, Some(&f), 2), StatusCode::OK);
    expect(api.mv(&k, Some(&p), 0), StatusCode::OK);
    assert_eq!(api.shape(), "[F](Alpha Parent(Kid) Zulu)");

    expect(
        api.admin("DELETE", &format!("/api/notes/{p}?base_version={version}")),
        StatusCode::OK,
    );
    assert_eq!(api.shape(), "[F](Alpha Kid Zulu)");

    expect(
        api.admin("POST", &format!("/api/trash/{p}/restore")),
        StatusCode::OK,
    );
    assert_eq!(api.shape(), "[F](Alpha Parent(Kid) Zulu)");
}
