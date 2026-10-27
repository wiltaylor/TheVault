//! S1: the sync handshake, the change feed over every kind of vault file, device writes that
//! always add a version, and base versions that survive pruning (D20, D27, D29).

use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, SET_COOKIE};
use serde_json::{Value, json};
use state::{PrunePolicy, State};
use testkit::{PASSWORD, TestServer};

const MEMO: &str = r#"label = "Memo"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const CARD: &str = r#"template card {
  tab "Card" {
    slot "title"
  }
}
"#;

const ANDROID: &str = "label = \"Android\"\n";

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
    std::fs::create_dir_all(vault.join("devices")).unwrap();
    std::fs::write(vault.join("devices/android.wcl"), ANDROID).unwrap();
}

fn with_header(name: reqwest::header::HeaderName, value: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(name, HeaderValue::from_str(value).unwrap());
    Client::builder().default_headers(headers).build().unwrap()
}

fn bearer(token: &str) -> Client {
    with_header(AUTHORIZATION, &format!("Bearer {token}"))
}

/// A client holding a browser session, as the web app is.
fn session(server: &TestServer) -> Client {
    let res = Client::new()
        .post(server.url("/api/login"))
        .json(&json!({ "password": PASSWORD }))
        .send()
        .unwrap();
    assert_eq!(res.status(), 200);
    let cookie = res.headers()[SET_COOKIE].to_str().unwrap();
    let pair = cookie.split(';').next().unwrap().to_owned();
    with_header(COOKIE, &pair)
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn send(rb: RequestBuilder, status: u16) -> Value {
    expect(rb.send().unwrap(), status)
}

fn create(c: &Client, server: &TestServer, title: &str) -> (String, String) {
    let note = send(
        c.post(server.url("/api/notes?type=memo"))
            .json(&json!({ "note": { "fields": { "title": title }, "blocks": [] } })),
        201,
    );
    (
        note["id"].as_str().unwrap().to_owned(),
        note["version"].as_str().unwrap().to_owned(),
    )
}

fn update(c: &Client, server: &TestServer, id: &str, base: &str, title: &str) -> String {
    let note = send(
        c.put(server.url(&format!("/api/notes/{id}"))).json(&json!({
            "note": { "fields": { "title": title }, "blocks": [] },
            "base_version": base,
        })),
        200,
    );
    note["version"].as_str().unwrap().to_owned()
}

fn history(c: &Client, server: &TestServer, id: &str) -> Vec<String> {
    let h = send(c.get(server.url(&format!("/api/notes/{id}/history"))), 200);
    h["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap().to_owned())
        .collect()
}

fn pair(server: &TestServer, name: &str) -> (String, String) {
    let out = send(
        Client::new()
            .post(server.url("/api/devices/pair"))
            .json(&json!({ "password": PASSWORD, "name": name, "platform": "android" })),
        201,
    );
    (
        out["device_id"].as_str().unwrap().to_owned(),
        out["token"].as_str().unwrap().to_owned(),
    )
}

fn changes(c: &Client, server: &TestServer, since: i64, limit: Option<u32>) -> Value {
    let mut url = format!("/api/sync/changes?since={since}");
    if let Some(limit) = limit {
        url.push_str(&format!("&limit={limit}"));
    }
    send(c.get(server.url(&url)), 200)
}

/// `(kind, id or path, deleted)` of each entry of a page.
fn summary(page: &Value) -> Vec<(String, String, bool)> {
    page["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let key = c["id"].as_str().or(c["path"].as_str()).unwrap().to_owned();
            (
                c["kind"].as_str().unwrap().to_owned(),
                key,
                c["deleted"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn open_state(server: &TestServer) -> State {
    State::open(&server.vault_path().join(".vault/state.db"), None).unwrap()
}

#[test]
fn sync_info_needs_no_credentials() {
    let server = TestServer::start();
    let info = send(Client::new().get(server.url("/api/sync/info")), 200);
    assert_eq!(info["protocol"], 1);
    assert_eq!(info["format_version"], 7);
    assert_eq!(info["server_version"], env!("CARGO_PKG_VERSION"));
    // The vault's id tells a device which vault its local copy belongs to (ticket 143).
    let vault_id = open_state(&server).vault_id().unwrap();
    assert_eq!(info["vault_id"], vault_id.as_str());
    // The feed itself needs credentials.
    send(Client::new().get(server.url("/api/sync/changes")), 401);
}

#[test]
fn the_server_version_needs_no_credentials() {
    let server = TestServer::start();
    let version = send(Client::new().get(server.url("/api/version")), 200);
    assert_eq!(version["server_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn the_change_feed_returns_every_kind_of_change_in_order_and_pages() {
    let server = TestServer::start_seeded(seed);
    let admin = bearer(server.admin_token());
    let (_, device_token) = pair(&server, "Pixel");
    let device = bearer(&device_token);

    // Files already in the vault when the server opened it are in the feed.
    let start = changes(&device, &server, 0, None);
    let first = summary(&start);
    assert!(
        first.contains(&("type".into(), "types/memo.wcl".into(), false)),
        "{start}"
    );
    assert!(
        first.contains(&("device".into(), "devices/android.wcl".into(), false)),
        "{start}"
    );
    let device_file = start["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == "devices/android.wcl")
        .unwrap();
    assert_eq!(device_file["text"], ANDROID);
    assert_eq!(device_file["name"], "android");
    let since = start["latest"].as_i64().unwrap();
    assert_eq!(start["next"], since);

    let (a, a1) = create(&admin, &server, "Alpha");
    let (b, b1) = create(&admin, &server, "Beta");
    let a2 = update(&admin, &server, &a, &a1, "Alpha two");
    let text = |rb: RequestBuilder, body: &str| {
        send(
            rb.header(CONTENT_TYPE, "text/plain").body(body.to_owned()),
            200,
        )
    };
    text(
        admin.put(server.url("/api/taxonomies/colour")),
        "label = \"Colour\"\n\nterm \"Red\"\n",
    );
    send(
        admin
            .put(server.url("/api/scripts/hello"))
            .json(&json!({ "source": "fn run() {}\n" })),
        200,
    );
    text(
        admin.put(server.url("/api/tasks/nightly")),
        "schedule = \"0 0 1 1 *\"\nscript = \"hello\"\n",
    );
    text(
        admin.put(server.url("/api/types/memo/templates/card")),
        CARD,
    );
    send(
        admin
            .delete(server.url(&format!("/api/notes/{b}")))
            .header("If-Match", format!("\"{b1}\"")),
        200,
    );
    send(admin.delete(server.url("/api/taxonomies/colour")), 200);

    let page = changes(&device, &server, since, None);
    assert_eq!(
        summary(&page),
        vec![
            ("note".into(), a.clone(), false),
            ("script".into(), "scripts/hello.ws".into(), false),
            ("task".into(), "tasks/nightly.wcl".into(), false),
            ("type".into(), "types/memo.wcl".into(), false),
            ("note".into(), b.clone(), true),
            ("taxonomy".into(), "taxonomies/colour.wcl".into(), true),
        ],
        "{page}"
    );
    let entries = page["changes"].as_array().unwrap();
    let seqs: Vec<i64> = entries.iter().map(|c| c["seq"].as_i64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    assert_eq!(page["next"], seqs[seqs.len() - 1]);
    assert_eq!(page["latest"], seqs[seqs.len() - 1]);

    // A note carries its current text and version; a trashed one says so.
    let read = send(admin.get(server.url(&format!("/api/notes/{a}"))), 200);
    assert_eq!(entries[0]["type"], "memo");
    assert_eq!(entries[0]["version"], a2.as_str());
    assert_eq!(entries[0]["wcl"], read["wcl"]);
    assert_eq!(entries[0]["trashed"], false);
    assert_eq!(entries[4]["trashed"], true);
    assert_eq!(entries[4]["type"], "memo");
    assert_eq!(entries[4]["version"], b1.as_str());
    assert!(entries[4]["wcl"].is_null(), "{}", entries[4]);
    assert!(
        entries[3]["text"]
            .as_str()
            .unwrap()
            .contains("template card")
    );
    assert_eq!(entries[1]["text"], "fn run() {}\n");
    assert_eq!(entries[1]["name"], "hello");
    assert!(entries[5]["text"].is_null());

    // Pages of two give the same entries, and the last page's `next` reaches `latest`.
    let mut cursor = since;
    let mut paged = Vec::new();
    loop {
        let p = changes(&device, &server, cursor, Some(2));
        let got = summary(&p);
        assert!(got.len() <= 2);
        if got.is_empty() {
            assert_eq!(p["next"], cursor);
            break;
        }
        paged.extend(got);
        cursor = p["next"].as_i64().unwrap();
        assert_eq!(p["latest"], page["latest"]);
        // `remaining` counts the entries still to come after the page (ticket 144).
        assert_eq!(
            p["remaining"].as_u64().unwrap() as usize,
            summary(&page).len() - paged.len(),
            "{p}"
        );
    }
    assert_eq!(paged, summary(&page));
    assert_eq!(cursor, page["latest"].as_i64().unwrap());

    // Restoring from the trash brings the note back at the end of the feed.
    send(
        admin.post(server.url(&format!("/api/trash/{b}/restore"))),
        200,
    );
    let after = changes(&device, &server, cursor, None);
    assert_eq!(summary(&after), vec![("note".into(), b.clone(), false)]);

    // Bad cursors and limits are refused.
    send(device.get(server.url("/api/sync/changes?since=-1")), 400);
    send(device.get(server.url("/api/sync/changes?limit=0")), 400);
    send(
        device.get(server.url("/api/sync/changes?limit=100000")),
        400,
    );
}

#[test]
fn device_writes_always_add_a_version_and_session_writes_still_merge() {
    let server = TestServer::start_seeded(seed);
    let browser = session(&server);
    let (_, device_token) = pair(&server, "Pixel");
    let device = bearer(&device_token);

    let (id, v1) = create(&browser, &server, "One");
    let v2 = update(&browser, &server, &id, &v1, "Two");
    assert_eq!(history(&browser, &server, &id), vec![v2.clone()]);

    // The device's write right after the browser's is a version of its own.
    let v3 = update(&device, &server, &id, &v2, "Three");
    assert_eq!(
        history(&browser, &server, &id),
        vec![v2.clone(), v3.clone()]
    );

    // A second device write is a new version again.
    let v4 = update(&device, &server, &id, &v3, "Four");
    assert_eq!(history(&browser, &server, &id).len(), 3);

    // The browser's next write does not merge into the device's version, but the one after it
    // merges into the browser's own.
    let v5 = update(&browser, &server, &id, &v4, "Five");
    let v6 = update(&browser, &server, &id, &v5, "Six");
    assert_eq!(
        history(&browser, &server, &id),
        vec![v2, v3, v4, v6.clone()]
    );
}

#[test]
fn a_recorded_base_survives_session_merges_and_pruning_until_the_device_clears_it() {
    let server = TestServer::start_seeded(seed);
    let admin = bearer(server.admin_token());
    let browser = session(&server);
    let (device_id, device_token) = pair(&server, "Pixel");
    let device = bearer(&device_token);
    let (_, other_token) = pair(&server, "Laptop");
    let bases_url = server.url(&format!("/api/devices/{device_id}/bases"));

    let (id, s1) = create(&browser, &server, "One");
    let out = send(
        device
            .put(&bases_url)
            .json(&json!({ "bases": { &id: &s1 } })),
        200,
    );
    assert_eq!(out["bases"], json!({ &id: &s1 }));

    // A browser save straight after would merge into s1; the base keeps it as it is.
    let s2 = update(&browser, &server, &id, &s1, "Two");
    let s3 = update(&browser, &server, &id, &s2, "Three");
    assert_eq!(
        history(&browser, &server, &id),
        vec![s1.clone(), s3.clone()]
    );
    let base = send(
        browser.get(server.url(&format!("/api/notes/{id}/history/{s1}"))),
        200,
    );
    assert!(base["wcl"].as_str().unwrap().contains("\"One\""), "{base}");

    // Pruning to one version per note keeps the base.
    let state = open_state(&server);
    let keep_one = PrunePolicy {
        keep_versions: Some(1),
        max_age_ms: None,
    };
    state.prune(&keep_one).unwrap();
    assert_eq!(
        history(&browser, &server, &id),
        vec![s1.clone(), s3.clone()]
    );
    let everything_old = PrunePolicy {
        keep_versions: None,
        max_age_ms: Some(0),
    };
    state.prune(&everything_old).unwrap();
    assert_eq!(
        history(&browser, &server, &id),
        vec![s1.clone(), s3.clone()]
    );

    // Only the device itself, or an admin, sets its bases; a base must be a version of its note.
    send(
        bearer(&other_token)
            .put(&bases_url)
            .json(&json!({ "bases": {} })),
        403,
    );
    send(
        device
            .put(&bases_url)
            .json(&json!({ "bases": { &id: "not-a-version" } })),
        400,
    );
    send(
        admin
            .put(server.url("/api/devices/01JB8Z4K2MQ7V3X9T5R6W8Y2ZA/bases"))
            .json(&json!({ "bases": {} })),
        404,
    );

    // Once the device clears it, pruning removes it.
    send(device.put(&bases_url).json(&json!({ "bases": {} })), 200);
    state.prune(&keep_one).unwrap();
    assert_eq!(history(&browser, &server, &id), vec![s3]);
    send(
        browser.get(server.url(&format!("/api/notes/{id}/history/{s1}"))),
        404,
    );
}

#[test]
fn revoking_a_device_forgets_its_bases() {
    let server = TestServer::start_seeded(seed);
    let admin = bearer(server.admin_token());
    let (device_id, device_token) = pair(&server, "Pixel");
    let device = bearer(&device_token);

    let (id, v1) = create(&admin, &server, "One");
    update(&admin, &server, &id, &v1, "Two");
    send(
        device
            .put(server.url(&format!("/api/devices/{device_id}/bases")))
            .json(&json!({ "bases": { &id: &v1 } })),
        200,
    );
    send(
        admin.delete(server.url(&format!("/api/devices/{device_id}"))),
        204,
    );
    open_state(&server)
        .prune(&PrunePolicy {
            keep_versions: Some(1),
            max_age_ms: None,
        })
        .unwrap();
    assert_eq!(history(&admin, &server, &id).len(), 1);
}

#[test]
fn device_tokens_cannot_write_definitions() {
    let server = TestServer::start_seeded(seed);
    let (_, device_token) = pair(&server, "Pixel");
    let device = bearer(&device_token);
    let text = |rb: RequestBuilder, body: &str| {
        rb.header(CONTENT_TYPE, "text/plain").body(body.to_owned())
    };
    for rb in [
        text(device.put(server.url("/api/types/other")), MEMO),
        device.delete(server.url("/api/types/memo")),
        text(
            device.put(server.url("/api/types/memo/templates/card")),
            CARD,
        ),
        text(
            device.put(server.url("/api/taxonomies/colour")),
            "label = \"Colour\"\n",
        ),
        device
            .put(server.url("/api/scripts/hello"))
            .json(&json!({ "source": "fn run() {}\n" })),
        text(
            device.put(server.url("/api/tasks/nightly")),
            "schedule = \"0 0 1 1 *\"\nscript = \"hello\"\n",
        ),
        device.delete(server.url("/api/tasks/nightly")),
    ] {
        assert_eq!(send(rb, 403)["error"], "forbidden");
    }
    // It reads them, and the feed, like any other token.
    send(device.get(server.url("/api/types/memo")), 200);
    send(device.get(server.url("/api/sync/changes")), 200);
}

#[test]
fn skill_files_are_in_the_feed_as_text_or_base64() {
    use base64::Engine as _;
    let server = TestServer::start();
    let admin = bearer(server.admin_token());
    let (_, token) = pair(&server, "Phone");
    let device = bearer(&token);
    let start = changes(&device, &server, 0, None)["latest"]
        .as_i64()
        .unwrap();
    let md = "---\nname: pdf\ndescription: Fills forms.\n---\n# PDF\n";
    send(
        admin
            .put(server.url("/api/skills/pdf"))
            .header(CONTENT_TYPE, "text/markdown")
            .body(md),
        201,
    );
    let blob: Vec<u8> = vec![0, 159, 146, 150, 255];
    send(
        admin
            .put(server.url("/api/skills/pdf/files/assets/blob.bin"))
            .body(blob.clone()),
        200,
    );
    let page = changes(&device, &server, start, None);
    assert_eq!(
        summary(&page),
        [
            ("skill".into(), "skills/pdf/SKILL.md".into(), false),
            ("skill".into(), "skills/pdf/assets/blob.bin".into(), false),
        ]
    );
    let entries = page["changes"].as_array().unwrap();
    assert_eq!(entries[0]["name"], "pdf");
    assert_eq!(entries[0]["text"], md);
    assert!(entries[0].get("base64").is_none());
    assert_eq!(entries[1]["text"], Value::Null);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(entries[1]["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(decoded, blob);
    // A device token reads skills and, as the apps edit them, writes them, but cannot import one
    // (skills S3).
    send(device.get(server.url("/api/skills/pdf")), 200);
    send(
        device
            .put(server.url("/api/skills/pdf"))
            .header(CONTENT_TYPE, "text/markdown")
            .body(md),
        200,
    );
    send(
        device.post(server.url("/api/skills/import")).body("PK"),
        403,
    );
    send(admin.delete(server.url("/api/skills/pdf")), 200);
    let next = page["next"].as_i64().unwrap();
    let mut gone = summary(&changes(&device, &server, next, None));
    gone.sort();
    assert_eq!(
        gone,
        [
            ("skill".into(), "skills/pdf/SKILL.md".into(), true),
            ("skill".into(), "skills/pdf/assets/blob.bin".into(), true),
        ]
    );
}
