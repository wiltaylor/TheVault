//! S1 with the fake provider and the fake web server: the chat's `fetch_url` and
//! `download_asset` tools, the downloads confirmation switch, and Apply of a download through
//! `POST /api/ai/apply` (ai D27). The fake web runs on 127.0.0.1, so these tests set
//! `ai_settings.fetch_private`, except the one that proves the address rule holds without it.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use testkit::{AnthropicMessage, FakeAnthropic, FakeWeb, Reply, TestServer};

const MODEL: &str = "claude-opus-5";

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start();
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Api { server, client }
    }

    fn token(&self) -> String {
        self.server.admin_token().to_owned()
    }

    fn secret(&self, name: &str, value: &str) {
        let res = self
            .client
            .put(self.server.url(&format!("/api/secrets/{name}")))
            .bearer_auth(self.token())
            .json(&json!({ "value": value }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    /// The fake Anthropic as the provider, with `confirm` and `fetch_private`.
    fn anthropic(&self, fake: &FakeAnthropic, confirm: Value, fetch_private: bool) {
        self.secret("ai_anthropic_api_key", fake.api_key());
        self.secret(
            "ai_settings",
            &json!({
                "default_provider": "anthropic",
                "models": { "anthropic": MODEL },
                "endpoints": { "anthropic": fake.base_url() },
                "fetch_private": fetch_private,
            })
            .to_string(),
        );
        // The chat's switches are the built-in Assistant's (agents G6).
        let mut all = json!({
            "notes": true, "archive": true, "delete": true,
            "types": true, "downloads": true, "taxonomies": true,
        });
        for (k, v) in confirm.as_object().into_iter().flatten() {
            all[k] = v.clone();
        }
        let res = self
            .client
            .put(self.server.url("/api/integrations/ai/settings"))
            .bearer_auth(self.token())
            .json(&json!({ "confirm": all }))
            .send()
            .unwrap();
        assert!(res.status().is_success(), "{}", res.text().unwrap());
    }

    fn token_of(&self, name: &str, scope: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/tokens"))
            .bearer_auth(self.token())
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: Value = res.json().unwrap();
        v["secret"].as_str().unwrap().to_owned()
    }

    /// One tool call as `token`, then an answer: the turn's events.
    fn call_as(
        &self,
        token: &str,
        fake: &FakeAnthropic,
        tool: &str,
        input: Value,
    ) -> Vec<(String, Value)> {
        fake.push(AnthropicMessage::new().tool_use("toolu_w", tool, input));
        fake.push(AnthropicMessage::text("Done."));
        let res = self
            .client
            .post(self.server.url("/api/ai/chat"))
            .bearer_auth(token)
            .json(&json!({ "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Go." }] }] }))
            .send()
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        read_events(res)
    }

    fn call(&self, fake: &FakeAnthropic, tool: &str, input: Value) -> Vec<(String, Value)> {
        self.call_as(&self.token(), fake, tool, input)
    }

    fn assets(&self) -> Vec<Value> {
        let v: Value = self
            .client
            .get(self.server.url("/api/assets"))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .json()
            .unwrap();
        v["assets"].as_array().unwrap().clone()
    }

    fn asset_bytes(&self, id: &str) -> Vec<u8> {
        self.client
            .get(self.server.url(&format!("/api/assets/{id}")))
            .bearer_auth(self.token())
            .send()
            .unwrap()
            .bytes()
            .unwrap()
            .to_vec()
    }

    fn apply_as(&self, token: &str, change: &Value) -> (StatusCode, Value) {
        let res = self
            .client
            .post(self.server.url("/api/ai/apply"))
            .bearer_auth(token)
            .json(change)
            .send()
            .unwrap();
        let status = res.status();
        (status, res.json().unwrap_or(Value::Null))
    }
}

fn read_events(res: Response) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    for line in BufReader::new(res).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty() {
                out.push((
                    std::mem::take(&mut name),
                    serde_json::from_str(&data).unwrap(),
                ));
                data.clear();
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("event:") {
            name = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push_str(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    out
}

/// The event names, without `usage`, with each run of `text` folded into one.
fn shape(events: &[(String, Value)]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for (n, _) in events {
        if n == "usage" || (n == "text" && out.last() == Some(&"text")) {
            continue;
        }
        out.push(n);
    }
    out
}

fn last<'a>(events: &'a [(String, Value)], name: &str) -> &'a Value {
    &events
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
        .1
}

/// The tool result the model got in the second request.
fn result(fake: &FakeAnthropic) -> Value {
    let r = fake.message_requests()[1].json();
    r["messages"].as_array().unwrap().last().unwrap()["content"][0].clone()
}

/// A tool result's text: its string content, or its first text block.
fn result_text(result: &Value) -> String {
    match &result["content"] {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks[0]["text"].as_str().unwrap().to_owned(),
        other => panic!("{other}"),
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn bytes_reply(media: &str, body: Vec<u8>) -> Reply {
    Reply {
        status: 200,
        headers: vec![("content-type".into(), media.into())],
        body,
        delay: Duration::ZERO,
    }
}

const ON: &str = r#"{}"#;

fn on() -> Value {
    serde_json::from_str(ON).unwrap()
}

#[test]
fn fetch_url_reads_a_pages_title_text_and_links_without_its_chrome() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, on(), true);
    // Through a redirect, so the final URL is the page's.
    let events = api.call(&fake, "fetch_url", json!({ "url": web.url("/redirect/1") }));
    assert_eq!(
        shape(&events),
        ["tool", "tool_result", "text", "done"],
        "{events:?}"
    );
    let r = result(&fake);
    assert_eq!(r["is_error"], false, "{r}");
    let text = result_text(&r);
    let page = web.url("/page.html");
    assert!(
        text.starts_with(&format!("URL: {page}\nMedia type: text/html\n")),
        "{text}"
    );
    assert!(text.contains("Title: Slate roofs: a guide"), "{text}");
    assert!(text.contains("not the user's"), "{text}");
    assert!(text.contains("# Why slate lasts"), "{text}");
    assert!(text.contains("Slate tiles last a hundred years."), "{text}");
    assert!(
        text.contains(&format!("[full specs]({})", web.url("/specs.html"))),
        "{text}"
    );
    assert!(
        text.contains("[history of slate](https://example.org/history)"),
        "{text}"
    );
    assert!(text.contains("- Durable\n- Fireproof"), "{text}");
    for chrome in [
        "SCRIPT-CONTENT",
        "NAV-CONTENT",
        "HEADER-CONTENT",
        "FOOTER-CONTENT",
        "ASIDE-CONTENT",
        "color: red",
    ] {
        assert!(!text.contains(chrome), "{chrome} in {text}");
    }
    // The panel learns the address, never the text.
    let summary = &last(&events, "tool_result")["summary"];
    assert_eq!(summary["web"]["url"], page.as_str());
    assert_eq!(summary["web"]["host"], "127.0.0.1");
    assert_eq!(summary["web"]["kind"], "html");
    assert_eq!(summary["web"]["title"], "Slate roofs: a guide");
    assert!(!summary.to_string().contains("hundred years"));
}

#[test]
fn fetch_url_reads_a_pdfs_text_and_cuts_a_long_text() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, on(), true);
    let pdf = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../index/tests/fixtures/three-pages.pdf"
    ))
    .unwrap();
    web.route("/report.pdf", bytes_reply("application/pdf", pdf));
    api.call(&fake, "fetch_url", json!({ "url": web.url("/report.pdf") }));
    let text = result_text(&result(&fake));
    assert!(text.contains("Media type: application/pdf"), "{text}");
    assert!(text.contains("zephyrine"), "{text}");

    // 6 MB of text: 5 MiB read, 40 000 characters kept, both said.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, on(), true);
    let size = 6_000_000;
    api.call(
        &fake,
        "fetch_url",
        json!({ "url": web.url(&format!("/bytes?size={size}")) }),
    );
    let text = result_text(&result(&fake));
    assert!(
        text.contains(&format!("Size: {size} bytes")),
        "{}",
        &text[..300]
    );
    assert!(
        text.contains("Read: the first 5242880 bytes only"),
        "{}",
        &text[..300]
    );
    assert!(
        text.ends_with("[truncated: the text is longer than this]"),
        "{}",
        &text[text.len() - 100..]
    );
    assert!(text.len() < 41_000, "{}", text.len());
}

#[test]
fn fetch_url_sends_an_image_as_an_image_part() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, on(), true);
    let bytes = png(32, 24);
    web.route("/photo.png", bytes_reply("image/png", bytes.clone()));
    let events = api.call(&fake, "fetch_url", json!({ "url": web.url("/photo.png") }));
    let images = fake.images();
    assert_eq!(images[1].len(), 1, "{images:?}");
    assert_eq!(images[1][0].tool_call_id, "toolu_w");
    assert_eq!(images[1][0].media_type, "image/png");
    assert_eq!(images[1][0].bytes, bytes);
    let done = last(&events, "done");
    let parts = &done["messages"][1]["content"][0]["parts"];
    assert_eq!(
        parts,
        &json!([{ "type": "image", "asset_id": "web:toolu_w", "media_type": "image/png" }])
    );
    assert_eq!(
        last(&events, "tool_result")["summary"]["web"]["kind"],
        "image"
    );
}

#[test]
fn the_address_rule_holds_without_fetch_private() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, json!({ "downloads": false }), false);
    let events = api.call(&fake, "fetch_url", json!({ "url": web.url("/page.html") }));
    let r = result(&fake);
    assert_eq!(r["is_error"], true, "{r}");
    assert!(result_text(&r).contains("TheVault does not fetch addresses on the local network."));
    assert_eq!(last(&events, "tool_result")["is_error"], true);

    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "downloads": false }), false);
    api.call(
        &fake,
        "download_asset",
        json!({ "url": web.url("/bytes?size=10"), "filename": null }),
    );
    assert!(result_text(&result(&fake)).contains("local network"));
    assert!(web.requests().is_empty());
    assert!(api.assets().is_empty());
}

#[test]
fn a_download_with_the_switch_on_is_a_proposal_that_apply_stores() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, on(), true);
    let bytes = png(10, 10);
    web.route("/img/sunset.png", bytes_reply("image/png", bytes.clone()));
    let events = api.call(
        &fake,
        "download_asset",
        json!({ "url": web.url("/img/sunset.png"), "filename": null }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "proposal").clone();
    assert_eq!(p["op"], "download");
    assert_eq!(p["id"], "toolu_w");
    assert_eq!(p["url"], web.url("/img/sunset.png").as_str());
    assert_eq!(p["filename"], "sunset.png");
    assert_eq!(p["media_type"], "image/png");
    assert_eq!(p["size"], bytes.len());
    assert!(result_text(&result(&fake)).contains("awaits the user"));
    // The probe was a HEAD; nothing was stored.
    assert_eq!(
        web.requests()
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["HEAD"]
    );
    assert!(api.assets().is_empty());

    // A read token cannot apply it; the admin can.
    let reader = api.token_of("reader", "read");
    let (status, _) = api.apply_as(&reader, &p);
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, done) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["op"], "download");
    assert_eq!(done["filename"], "sunset.png");
    let id = done["asset_id"].as_str().unwrap();
    assert_eq!(api.assets().len(), 1);
    assert_eq!(api.asset_bytes(id), bytes);
}

#[test]
fn a_download_with_the_switch_off_is_stored_at_once() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, json!({ "downloads": false }), true);
    let events = api.call(
        &fake,
        "download_asset",
        json!({ "url": web.url("/bytes?size=300&type=text/plain"), "filename": "notes/today" }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "applied", "tool_result", "text", "done"],
        "{events:?}"
    );
    let a = last(&events, "applied");
    assert_eq!(a["op"], "download");
    assert_eq!(a["id"], "toolu_w");
    assert_eq!(a["filename"], "notes_today.txt");
    let id = a["asset_id"].as_str().unwrap();
    let content: Value = serde_json::from_str(&result_text(&result(&fake))).unwrap();
    assert_eq!(content["asset_id"], id);
    assert_eq!(content["applied"], true);
    let assets = api.assets();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["filename"], "notes_today.txt");
    assert_eq!(api.asset_bytes(id).len(), 300);
}

/// Whether `body` is the fake web's generated body of its length: one line over and over.
fn is_generated(body: &[u8]) -> bool {
    const LINE: &[u8] = b"The quick brown fox jumps over the lazy dog. 0123456789\n";
    body.iter()
        .enumerate()
        .all(|(i, b)| *b == LINE[i % LINE.len()])
}

/// The files left in the vault's `.vault/incoming`, where downloads are written.
fn incoming(api: &Api) -> Vec<String> {
    std::fs::read_dir(api.server.vault_path().join(".vault/incoming"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_download_has_no_size_limit() {
    let api = Api::start();
    let web = FakeWeb::start();
    // Larger than the 25 MiB the downloads were once limited to.
    let size: u64 = 30 * 1024 * 1024;

    // Proposed, with its size, then applied.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, on(), true);
    let url = web.url(&format!("/bytes?size={size}&type=application/zip"));
    let events = api.call(
        &fake,
        "download_asset",
        json!({ "url": url, "filename": "big.zip" }),
    );
    let p = last(&events, "proposal").clone();
    assert_eq!(p["size"], size, "{p}");
    let (status, done) = api.apply_as(&api.token(), &p);
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["size"], size);
    let body = api.asset_bytes(done["asset_id"].as_str().unwrap());
    assert_eq!(body.len() as u64, size);
    assert!(is_generated(&body));

    // Made at once, from a body with no Content-Length.
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "downloads": false }), true);
    let events = api.call(
        &fake,
        "download_asset",
        json!({ "url": web.url(&format!("/bytes?size={size}&nolength=1")), "filename": "big.txt" }),
    );
    let a = last(&events, "applied");
    assert_eq!(a["size"], size, "{a}");
    let body = api.asset_bytes(a["asset_id"].as_str().unwrap());
    assert_eq!(body.len() as u64, size);
    assert!(is_generated(&body));
    assert_eq!(api.assets().len(), 2);
    assert!(incoming(&api).is_empty(), "{:?}", incoming(&api));
}

#[test]
fn a_download_cut_off_part_way_stores_nothing_and_leaves_no_file() {
    let api = Api::start();
    let web = FakeWeb::start();
    // The connection closes after 1 MiB of the 4 MiB it promised.
    let url = web.url("/bytes?size=4194304&drop=1048576");

    let fake = FakeAnthropic::start();
    api.anthropic(&fake, json!({ "downloads": false }), true);
    let events = api.call(
        &fake,
        "download_asset",
        json!({ "url": url, "filename": "cut.bin" }),
    );
    let r = result(&fake);
    assert_eq!(r["is_error"], true, "{r}");
    assert_eq!(shape(&events), ["tool", "tool_result", "text", "done"]);
    assert!(api.assets().is_empty());
    assert!(incoming(&api).is_empty(), "{:?}", incoming(&api));

    let (status, body) = api.apply_as(
        &api.token(),
        &json!({ "op": "download", "url": url, "filename": "cut.bin" }),
    );
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"], "fetch_failed", "{body}");
    assert!(api.assets().is_empty());
    assert!(incoming(&api).is_empty(), "{:?}", incoming(&api));
}

#[test]
fn a_read_scope_caller_always_gets_a_download_proposal() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    api.anthropic(&fake, json!({ "downloads": false }), true);
    let reader = api.token_of("reader", "read");
    let events = api.call_as(
        &reader,
        &fake,
        "download_asset",
        json!({ "url": web.url("/bytes?size=50&nohead=1"), "filename": "a.txt" }),
    );
    assert_eq!(
        shape(&events),
        ["tool", "proposal", "tool_result", "text", "done"],
        "{events:?}"
    );
    let p = last(&events, "proposal");
    assert_eq!(p["size"], 50);
    assert_eq!(p["filename"], "a.txt");
    // HEAD refused, so the probe was a ranged GET.
    let methods: Vec<String> = web.requests().iter().map(|r| r.method.clone()).collect();
    assert_eq!(methods, ["HEAD", "GET"]);
    assert!(api.assets().is_empty());
}

#[test]
fn the_downloads_switch_defaults_to_on_and_saves() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    api.anthropic(&fake, on(), true);
    let res = api
        .client
        .put(api.server.url("/api/integrations/ai/settings"))
        .bearer_auth(api.token())
        .json(&json!({ "confirm": { "downloads": false } }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: Value = res.json().unwrap();
    assert_eq!(v["confirm"]["downloads"], false);
    assert_eq!(v["confirm"]["notes"], true);
    let p: Value = api
        .client
        .get(api.server.url("/api/ai/providers"))
        .bearer_auth(api.token())
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(p["confirm"]["downloads"], false);
}

/// The tool names the first request of the last turn offered the model.
fn offered(fake: &FakeAnthropic) -> Vec<String> {
    let reqs = fake.message_requests();
    let first: Value = serde_json::from_slice(&reqs[reqs.len() - 2].body).unwrap();
    first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_internet_switches_leave_their_tools_out_and_refuse_a_call_to_them() {
    let api = Api::start();
    let fake = FakeAnthropic::start();
    let web = FakeWeb::start();
    // On by default: both web tools are offered.
    api.anthropic(&fake, on(), true);
    api.call(&fake, "fetch_url", json!({ "url": web.url("/page.html") }));
    let tools = offered(&fake);
    assert!(tools.iter().any(|t| t == "fetch_url"), "{tools:?}");
    assert!(tools.iter().any(|t| t == "download_asset"), "{tools:?}");

    // Off through the settings route: left out, and a call naming one is refused unrun.
    let res = api
        .client
        .put(api.server.url("/api/integrations/ai/settings"))
        .bearer_auth(api.token())
        .json(&json!({ "web": { "read": false, "download": false } }))
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let status: Value = res.json().unwrap();
    assert_eq!(
        status["web"],
        json!({ "search": true, "read": false, "download": false })
    );

    let before = web.requests().len();
    api.call(&fake, "fetch_url", json!({ "url": web.url("/page.html") }));
    let tools = offered(&fake);
    assert!(
        !tools
            .iter()
            .any(|t| t == "fetch_url" || t == "download_asset"),
        "{tools:?}"
    );
    let reqs = fake.message_requests();
    let last = reqs.last().unwrap().json();
    let r = last["messages"].as_array().unwrap().last().unwrap()["content"][0].clone();
    assert_eq!(r["is_error"], true, "{r}");
    assert_eq!(
        web.requests().len(),
        before,
        "a switched-off tool fetched nothing"
    );
}
