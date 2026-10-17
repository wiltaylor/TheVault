//! S1 checks for the assets library: listing with use counts, filters and pages, the notes using
//! an asset, cleaning up the unused ones with a dry run first, the asset trash, and renames.

use std::path::Path;

use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const DOC: &str = r#"label = "Doc"

field file {
  kind = "asset"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "file"
    slot "body"
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

fn upload(server: &TestServer, c: &Client, name: &str, bytes: &[u8]) -> String {
    let res = c
        .post(server.url(&format!("/api/assets?filename={name}")))
        .body(bytes.to_vec())
        .send()
        .unwrap();
    expect(res, 201)["id"].as_str().unwrap().to_owned()
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
fn the_library_lists_uses_and_cleans_up_what_nothing_uses() {
    let server = TestServer::start_seeded(seed);
    let c = client(server.admin_token());
    let photo = upload(&server, &c, "photo.png", b"png");
    let inline = upload(&server, &c, "inline.png", b"png2");
    let spare = upload(&server, &c, "spare.zip", b"zip bytes");
    let res = c
        .post(server.url("/api/notes?type=doc"))
        .json(&json!({ "note": { "fields": { "title": "Holiday", "file": photo } } }))
        .send()
        .unwrap();
    let created = expect(res, 201);
    let note = created["id"].as_str().unwrap().to_owned();
    let version = created["version"].as_str().unwrap().to_owned();
    let res = c
        .post(server.url("/api/notes?type=doc"))
        .json(
            &json!({ "note": { "fields": { "title": "Inline" }, "blocks": [
            { "kind": "content", "labels": ["body"], "blocks": [
                { "kind": "image", "fields": { "asset": inline } }
            ] }
        ] } }),
        )
        .send()
        .unwrap();
    let inline_note = expect(res, 201)["id"].as_str().unwrap().to_owned();

    let all = expect(
        c.get(server.url("/api/assets?sort=name")).send().unwrap(),
        200,
    );
    assert_eq!(names(&all), ["inline.png", "photo.png", "spare.zip"]);
    assert_eq!(all["total"], 3);
    assert_eq!(all["counts"]["image"], 2);
    assert_eq!(all["counts"]["other"], 1);
    assert_eq!(all["unused"], json!({ "count": 1, "size": 9 }));
    let first = &all["assets"][1];
    assert_eq!(first["used_by"], 1);
    assert_eq!(first["kind"], "image");
    assert!(first["uploaded_ms"].as_u64().unwrap() > 1_600_000_000_000);

    let unused = expect(
        c.get(server.url("/api/assets?unused=true")).send().unwrap(),
        200,
    );
    assert_eq!(names(&unused), ["spare.zip"]);
    let images = expect(
        c.get(server.url("/api/assets?kind=image&q=PHO"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(names(&images), ["photo.png"]);
    let page = expect(
        c.get(server.url("/api/assets?sort=name&limit=2"))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(names(&page), ["inline.png", "photo.png"]);
    let next = page["next"].as_str().unwrap();
    let rest = expect(
        c.get(server.url(&format!("/api/assets?sort=name&limit=2&cursor={next}")))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(names(&rest), ["spare.zip"]);
    assert!(rest["next"].is_null());

    let usage = expect(
        c.get(server.url(&format!("/api/assets/{inline}/usage")))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(usage["filename"], "inline.png");
    assert_eq!(usage["notes"][0]["id"], inline_note);
    assert_eq!(usage["notes"][0]["title"], "Inline");
    assert_eq!(
        usage["notes"][0]["how"],
        json!([{ "field": "body", "via": "image" }])
    );

    // A dry run deletes nothing; the cleanup then takes only the unused one.
    let dry = expect(
        c.post(server.url("/api/assets/cleanup"))
            .json(&json!({ "dry_run": true }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(dry["count"], 1);
    assert_eq!(dry["size"], 9);
    assert!(server.vault_path().join("assets").join(&spare).exists());
    let done = expect(
        c.post(server.url("/api/assets/cleanup"))
            .json(&json!({ "ids": [spare, photo] }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(done["count"], 1);
    assert!(!server.vault_path().join("assets").join(&spare).exists());
    assert!(server.vault_path().join("assets").join(&photo).exists());

    let trash = expect(c.get(server.url("/api/assets/trash")).send().unwrap(), 200);
    assert_eq!(trash["assets"][0]["id"], spare);
    let back = expect(
        c.post(server.url("/api/assets/trash/restore"))
            .json(&json!({ "ids": [spare] }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(back["restored"][0]["filename"], "spare.zip");
    let bytes = c
        .get(server.url(&format!("/api/assets/{spare}")))
        .send()
        .unwrap()
        .bytes()
        .unwrap();
    assert_eq!(&bytes[..], b"zip bytes");

    // A note in the trash keeps its asset.
    expect(
        c.delete(server.url(&format!("/api/notes/{note}?base_version={version}")))
            .send()
            .unwrap(),
        200,
    );
    let refused = expect(
        c.delete(server.url(&format!("/api/assets/{photo}?unlink=true")))
            .send()
            .unwrap(),
        409,
    );
    assert_eq!(refused["error"], "asset_in_trash");
    assert_eq!(refused["notes"][0]["id"], note);
    assert_eq!(refused["notes"][0]["title"], "Holiday");
    assert_eq!(refused["notes"][0]["trashed"], true);
    let usage = expect(
        c.get(server.url(&format!("/api/assets/{photo}/usage")))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(usage["trashed"][0]["title"], "Holiday");
    assert_eq!(usage["used_by"], 1);

    let renamed = expect(
        c.patch(server.url(&format!("/api/assets/{spare}")))
            .json(&json!({ "filename": "archive.zip" }))
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(renamed["path"], format!("assets/{spare}/archive.zip"));
}
