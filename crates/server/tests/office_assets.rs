//! S1 checks for Word documents, spreadsheets and CSV files: served with their own media
//! types, listed as the `document` and `spreadsheet` kinds with filters and counts, and their
//! text found by full-text search through the notes that name them and by the library search.

use std::path::Path;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

#[path = "../../index/tests/support/office.rs"]
mod office;

const DOC: &str = r#"label = "Doc"

field file {
  kind = "asset"
}

template default {
  tab "Main" {
    slot "title"
    slot "file"
  }
}
"#;

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/doc.wcl"), DOC).unwrap();
}

fn client(token: &str) -> Client {
    let mut auth = HeaderMap::new();
    auth.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(auth).build().unwrap()
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "body: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn upload(server: &TestServer, c: &Client, name: &str, bytes: Vec<u8>) -> String {
    let res = c
        .post(server.url(&format!("/api/assets?filename={name}")))
        .body(bytes)
        .send()
        .unwrap();
    expect(res, 201)["id"].as_str().unwrap().to_owned()
}

fn note(server: &TestServer, c: &Client, title: &str, file: &str) -> String {
    let res = c
        .post(server.url("/api/notes?type=doc"))
        .json(&json!({ "note": { "fields": { "title": title, "file": file } } }))
        .send()
        .unwrap();
    expect(res, 201)["id"].as_str().unwrap().to_owned()
}

fn search(server: &TestServer, c: &Client, q: &str) -> Vec<String> {
    let found = expect(
        c.get(server.url(&format!("/api/search?q={q}")))
            .send()
            .unwrap(),
        200,
    );
    found["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{found}"))
        .iter()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

fn names(list: &Value) -> Vec<String> {
    list["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["filename"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn documents_and_spreadsheets_are_their_own_kinds_and_searchable() {
    let server = TestServer::start_seeded(seed);
    let c = client(server.admin_token());
    let docx = upload(&server, &c, "Report.docx", office::sample_docx());
    let xlsx = upload(&server, &c, "Budget.xlsx", office::sample_xlsx());
    let csv = upload(
        &server,
        &c,
        "people.csv",
        b"name,town\nAda,Puddlecombe\n".to_vec(),
    );
    upload(&server, &c, "song.mp3", b"mp3".to_vec());
    let in_docx = note(&server, &c, "Report", &docx);
    let in_xlsx = note(&server, &c, "Budget", &xlsx);

    assert_eq!(search(&server, &c, "quillwort"), [in_docx]);
    assert_eq!(search(&server, &c, "marshwiggle"), [in_xlsx]);

    for (id, ty) in [
        (
            &docx,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        (
            &xlsx,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        (&csv, "text/csv; charset=utf-8"),
    ] {
        let res = c
            .get(server.url(&format!("/api/assets/{id}")))
            .send()
            .unwrap();
        assert_eq!(res.headers()["content-type"], ty);
    }

    let all = expect(
        c.get(server.url("/api/assets?sort=name")).send().unwrap(),
        200,
    );
    let kinds: Vec<&str> = all["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["spreadsheet", "spreadsheet", "document", "audio"]);
    assert_eq!(all["counts"]["document"], 1);
    assert_eq!(all["counts"]["spreadsheet"], 2);
    assert_eq!(all["counts"]["other"], 1);
    let sheets = expect(
        c.get(server.url("/api/assets?kind=spreadsheet&sort=name"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(names(&sheets), ["Budget.xlsx", "people.csv"]);

    // The library search reads the text of each kind.
    for (q, name) in [
        ("quillwort", "Report.docx"),
        ("marshwiggle", "Budget.xlsx"),
        ("puddlecombe", "people.csv"),
    ] {
        let found = expect(
            c.get(server.url(&format!("/api/assets?q={q}")))
                .send()
                .unwrap(),
            200,
        );
        assert_eq!(names(&found), [name], "{q}");
    }
}
