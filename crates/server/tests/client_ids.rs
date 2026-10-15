//! S1: notes and assets created under an id the client chose, as a device pushes what it made
//! offline (D19, D20), and the `conflict_of` field of a conflict copy.

use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const MEMO: &str = r#"label = "Memo"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const ID: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZA";
const OTHER: &str = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

fn admin(server: &TestServer) -> Client {
    let mut headers = HeaderMap::new();
    let value = format!("Bearer {}", server.admin_token());
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&value).unwrap());
    Client::builder().default_headers(headers).build().unwrap()
}

fn send(rb: RequestBuilder, status: u16) -> Value {
    let res = rb.send().unwrap();
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn note(title: &str) -> Value {
    json!({ "note": { "fields": { "title": title }, "blocks": [] } })
}

#[test]
fn a_note_is_created_under_the_id_the_client_chose_once() {
    let server = TestServer::start_seeded(seed);
    let c = admin(&server);
    let put = |id: &str| {
        c.put(server.url(&format!("/api/notes/{id}?type=memo")))
            .header("If-None-Match", "*")
            .json(&note("Offline"))
    };
    let made = send(put(ID), 201);
    assert_eq!(made["id"], ID);
    assert_eq!(made["event"], "create");
    let read = send(c.get(server.url(&format!("/api/notes/{ID}"))), 200);
    assert_eq!(read["note"]["fields"]["title"], "Offline");

    // Taken while live, and while in the trash.
    assert_eq!(send(put(ID), 409)["error"], "id_taken");
    send(
        c.delete(server.url(&format!("/api/notes/{ID}"))).header(
            "If-Match",
            format!("\"{}\"", made["version"].as_str().unwrap()),
        ),
        200,
    );
    assert_eq!(send(put(ID), 409)["error"], "id_taken");
    // Not an id, and no type.
    send(put("not-a-ulid"), 400);
    send(
        c.put(server.url(&format!("/api/notes/{OTHER}")))
            .header("If-None-Match", "*")
            .json(&note("No type")),
        400,
    );
    // Without If-None-Match, a note that does not exist is still not found.
    send(
        c.put(server.url(&format!("/api/notes/{OTHER}")))
            .header("If-Match", "\"x\"")
            .json(&note("Missing")),
        404,
    );
}

#[test]
fn an_asset_is_stored_under_the_id_the_client_chose() {
    let server = TestServer::start_seeded(seed);
    let c = admin(&server);
    let put = |bytes: &[u8], name: &str| {
        c.put(server.url(&format!("/api/assets/{ID}")))
            .query(&[("filename", name)])
            .body(bytes.to_vec())
    };
    assert_eq!(send(put(b"one", "a.txt"), 201)["id"], ID);
    // The same file again is fine; anything else under the id is not.
    send(put(b"one", "a.txt"), 200);
    assert_eq!(send(put(b"two", "a.txt"), 409)["error"], "id_taken");
    let got = c
        .get(server.url(&format!("/api/assets/{ID}")))
        .send()
        .unwrap();
    assert_eq!(got.bytes().unwrap().as_ref(), b"one");
}

#[test]
fn a_conflict_copy_names_its_original() {
    let server = TestServer::start_seeded(seed);
    let c = admin(&server);
    let original = send(
        c.post(server.url("/api/notes?type=memo"))
            .json(&note("Plan")),
        201,
    );
    let id = original["id"].as_str().unwrap();
    let copy = send(
        c.post(server.url("/api/notes?type=memo")).json(&json!({
            "note": { "fields": { "title": "Plan (conflict, Pixel)", "conflict_of": id } },
        })),
        201,
    );
    assert_eq!(copy["note"]["fields"]["conflict_of"], id);
    assert!(
        copy["wcl"]
            .as_str()
            .unwrap()
            .contains(&format!("conflict_of = \"{id}\"")),
        "{copy}"
    );
    // The copy links to the original.
    let graph = send(c.get(server.url(&format!("/api/notes/{id}/graph"))), 200);
    assert!(
        graph.to_string().contains(copy["id"].as_str().unwrap()),
        "{graph}"
    );
    // It must name a note id; a type cannot define a field of that name.
    let bad = send(
        c.post(server.url("/api/notes?type=memo")).json(&json!({
            "note": { "fields": { "title": "Bad", "conflict_of": "nope" } },
        })),
        400,
    );
    assert_eq!(bad["error"], "invalid");
    let reserved = send(
        c.put(server.url("/api/types/clash"))
            .header("Content-Type", "text/plain")
            .body(format!(
                "{MEMO}\nfield conflict_of {{\n  kind = \"string\"\n}}\n"
            )),
        400,
    );
    assert!(reserved.to_string().contains("reserved"), "{reserved}");
}
