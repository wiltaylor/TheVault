//! S1 checks for PDF assets: served as `application/pdf` with byte ranges for pdf.js, their
//! text found by full-text search through the notes that name them, and a corrupt PDF still
//! uploaded.

use std::path::Path;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, RANGE};
use serde_json::{Value, json};
use testkit::TestServer;

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

fn pdf() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../index/tests/fixtures/three-pages.pdf"
    ))
    .unwrap()
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

#[test]
fn a_pdf_is_served_as_pdf_and_in_byte_ranges() {
    let server = TestServer::start_seeded(seed);
    let c = client(server.admin_token());
    let bytes = pdf();
    let id = upload(&server, &c, "sample.pdf", bytes.clone());
    let url = server.url(&format!("/api/assets/{id}"));

    let res = c.get(&url).send().unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers();
    assert_eq!(h["content-type"], "application/pdf");
    assert_eq!(h["accept-ranges"], "bytes");
    // The browser's PDF viewer will not run sandboxed.
    assert!(h.get("content-security-policy").is_none());
    assert_eq!(res.bytes().unwrap().to_vec(), bytes);

    let res = c.get(&url).header(RANGE, "bytes=10-19").send().unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()["content-range"],
        format!("bytes 10-19/{}", bytes.len()).as_str()
    );
    assert_eq!(res.headers()["content-type"], "application/pdf");
    assert_eq!(res.bytes().unwrap().to_vec(), bytes[10..20].to_vec());

    let res = c.get(&url).header(RANGE, "bytes=-6").send().unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.bytes().unwrap().to_vec(),
        bytes[bytes.len() - 6..].to_vec()
    );

    let past = format!("bytes={}-", bytes.len());
    let res = c.get(&url).header(RANGE, past).send().unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        res.headers()["content-range"],
        format!("bytes */{}", bytes.len()).as_str()
    );

    // Any other file keeps its sandbox.
    let txt = upload(&server, &c, "a.txt", b"hello".to_vec());
    let res = c
        .get(server.url(&format!("/api/assets/{txt}")))
        .send()
        .unwrap();
    let csp = res.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("sandbox"), "{csp}");
}

#[test]
fn search_finds_a_note_by_a_word_only_inside_its_pdf() {
    let server = TestServer::start_seeded(seed);
    let c = client(server.admin_token());
    let id = upload(&server, &c, "sample.pdf", pdf());
    let res = c
        .post(server.url("/api/notes?type=doc"))
        .json(&json!({ "note": { "fields": { "title": "Quarterly", "file": id } } }))
        .send()
        .unwrap();
    let note = expect(res, 201)["id"].as_str().unwrap().to_owned();

    let found = expect(
        c.get(server.url("/api/search?q=zephyrine")).send().unwrap(),
        200,
    );
    let ids: Vec<&str> = found["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{found}"))
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(ids, [note.as_str()], "{found}");

    // A corrupt PDF uploads, downloads byte for byte, and breaks no search.
    let broken = b"%PDF-1.7\n1 0 obj << /Pages 9 0 R >>\n%%EOF".to_vec();
    let bad = upload(&server, &c, "broken.pdf", broken.clone());
    let res = c
        .get(server.url(&format!("/api/assets/{bad}")))
        .send()
        .unwrap();
    assert_eq!(res.bytes().unwrap().to_vec(), broken);
    let res = c
        .post(server.url("/api/notes?type=doc"))
        .json(&json!({ "note": { "fields": { "title": "Broken", "file": bad } } }))
        .send()
        .unwrap();
    expect(res, 201);
    expect(
        c.get(server.url("/api/search?q=zephyrine")).send().unwrap(),
        200,
    );
}
