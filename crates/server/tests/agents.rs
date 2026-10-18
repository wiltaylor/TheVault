//! S1: connecting an AI agent. An admin issues a one-time install code, the agent's install
//! script redeems it for an `agent-<name>` token with the chosen scope, and `GET /agent/install.sh`
//! serves that script with the server's URL in it (D11, D32, ticket 131).

use std::ffi::OsStr;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use testkit::TestServer;

const NOTE_TYPE: &str = r#"label = "Memo"

template default {
  tab "Main" {
    slot "title"
  }
}
"#;

const START: &str = "2026-01-05T00:30:00Z";

fn seed(vault: &std::path::Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), NOTE_TYPE).unwrap();
}

fn client(token: &str) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
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

fn redeem(server: &TestServer, code: &str) -> Response {
    Client::new()
        .post(server.url("/api/agent-codes/redeem"))
        .json(&json!({ "code": code }))
        .send()
        .unwrap()
}

fn issue(server: &TestServer, admin: &Client, name: &str, scope: &str) -> Value {
    send(
        admin
            .post(server.url("/api/agent-codes"))
            .json(&json!({ "name": name, "scope": scope })),
        201,
    )
}

#[test]
fn only_an_admin_issues_an_install_code_and_it_carries_the_one_line_command() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());

    let anon = Client::new()
        .post(server.url("/api/agent-codes"))
        .json(&json!({ "name": "claude", "scope": "write" }))
        .send()
        .unwrap();
    expect(anon, 401);
    let made = send(
        admin
            .post(server.url("/api/tokens"))
            .json(&json!({ "name": "writer", "scope": "write" })),
        201,
    );
    let writer = client(made["secret"].as_str().unwrap());
    send(
        writer
            .post(server.url("/api/agent-codes"))
            .json(&json!({ "name": "claude", "scope": "read" })),
        403,
    );

    let issued = issue(&server, &admin, "claude", "write");
    let code = issued["code"].as_str().unwrap();
    let base = server.base_url();
    assert_eq!(code.len(), 9, "{code}");
    assert_eq!(issued["name"], "agent-claude");
    assert_eq!(issued["scope"], "write");
    assert_eq!(issued["server_url"], base.as_str());
    assert_eq!(
        issued["command"],
        format!("curl -fsSL {base}/agent/install.sh | sh -s -- {code}")
    );
    let ttl = issued["expires"].as_i64().unwrap() - chrono_now_ms();
    assert!((590_000..=600_000).contains(&ttl), "{ttl}");

    // The browser names the address it reaches the server on.
    let named = send(
        admin.post(server.url("/api/agent-codes")).json(
            &json!({ "name": "codex", "scope": "read", "server_url": "https://vault.example/" }),
        ),
        201,
    );
    assert!(
        named["command"]
            .as_str()
            .unwrap()
            .starts_with("curl -fsSL https://vault.example/agent/install.sh | sh -s -- "),
        "{named}"
    );

    for bad in [
        json!({ "name": "claude", "scope": "owner" }),
        json!({ "name": "", "scope": "write" }),
        json!({ "name": "has space", "scope": "write" }),
        json!({ "name": "x".repeat(60), "scope": "write" }),
        json!({ "name": "claude", "scope": "write", "server_url": "ftp://x" }),
        json!({ "name": "claude", "scope": "write", "server_url": "https://a'b" }),
    ] {
        send(admin.post(server.url("/api/agent-codes")).json(&bad), 400);
    }
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[test]
fn a_code_redeems_once_for_a_token_of_its_scope_and_reuse_is_gone() {
    let server = TestServer::start_seeded(seed);
    let admin = client(server.admin_token());

    let issued = issue(&server, &admin, "claude", "write");
    let code = issued["code"].as_str().unwrap();
    let out = expect(redeem(&server, code), 201);
    assert_eq!(out["name"], "agent-claude");
    assert_eq!(out["scope"], "write");
    assert_eq!(out["server_url"], server.base_url().as_str());
    let token = out["token"].as_str().unwrap();
    assert!(token.starts_with("tv_"), "{out}");

    // Used up.
    assert_eq!(expect(redeem(&server, code), 410)["error"], "gone");
    expect(redeem(&server, "0000-0000"), 410);

    // A write token writes notes and reaches nothing admin.
    let agent = client(token);
    send(
        agent
            .post(server.url("/api/notes?type=memo"))
            .json(&json!({ "note": { "fields": { "title": "From the agent" }, "blocks": [] } })),
        201,
    );
    send(agent.get(server.url("/api/notes")), 200);
    send(agent.get(server.url("/api/tokens")), 403);
    send(
        agent
            .put(server.url("/api/types/other"))
            .json(&json!({ "wcl": NOTE_TYPE })),
        403,
    );
    send(
        agent
            .post(server.url("/api/agent-codes"))
            .json(&json!({ "name": "more", "scope": "admin" })),
        403,
    );

    // A read token reads only; an admin one reaches admin.
    let read = expect(
        redeem(
            &server,
            issue(&server, &admin, "reader", "read")["code"]
                .as_str()
                .unwrap(),
        ),
        201,
    );
    let reader = client(read["token"].as_str().unwrap());
    send(reader.get(server.url("/api/notes")), 200);
    send(
        reader
            .post(server.url("/api/notes?type=memo"))
            .json(&json!({ "note": { "fields": { "title": "No" }, "blocks": [] } })),
        403,
    );
    let full = expect(
        redeem(
            &server,
            issue(&server, &admin, "boss", "admin")["code"]
                .as_str()
                .unwrap(),
        ),
        201,
    );
    send(
        client(full["token"].as_str().unwrap()).get(server.url("/api/tokens")),
        200,
    );

    // The tokens are listed with the others and revoke like them.
    let listed = send(admin.get(server.url("/api/tokens")), 200);
    let names: Vec<&str> = listed["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in ["agent-claude", "agent-reader", "agent-boss"] {
        assert!(names.contains(&name), "{names:?}");
    }
    send(admin.delete(server.url("/api/tokens/agent-claude")), 204);
    send(agent.get(server.url("/api/notes")), 401);

    // A new code for the same name replaces its token.
    let again = expect(
        redeem(
            &server,
            issue(&server, &admin, "reader", "write")["code"]
                .as_str()
                .unwrap(),
        ),
        201,
    );
    send(reader.get(server.url("/api/notes")), 401);
    send(
        client(again["token"].as_str().unwrap()).get(server.url("/api/notes")),
        200,
    );
}

#[test]
fn a_code_older_than_ten_minutes_is_gone() {
    let server =
        TestServer::start_with_args(seed, &[OsStr::new("--test-clock"), OsStr::new(START)]);
    let admin = client(server.admin_token());
    let advance = |by: &str| {
        send(
            admin
                .post(server.url("/api/test/clock"))
                .json(&json!({ "advance": by })),
            200,
        );
    };

    let fresh = issue(&server, &admin, "fresh", "write");
    let stale = issue(&server, &admin, "stale", "write");
    advance("PT9M");
    expect(redeem(&server, fresh["code"].as_str().unwrap()), 201);
    advance("PT2M");
    let late = expect(redeem(&server, stale["code"].as_str().unwrap()), 410);
    assert!(
        late["message"].as_str().unwrap().contains("expired"),
        "{late}"
    );
}

#[test]
fn the_install_script_is_served_to_anyone_with_the_server_url_in_it() {
    let server = TestServer::start_seeded(seed);
    let res = Client::new()
        .get(server.url("/agent/install.sh"))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let kind = res.headers()["content-type"].to_str().unwrap().to_owned();
    assert!(kind.starts_with("text/x-shellscript"), "{kind}");
    let script = res.text().unwrap();
    assert!(script.starts_with("#!/bin/sh\n"), "{script}");
    assert!(
        script.contains(&format!("VAULT_URL='{}'", server.base_url())),
        "{script}"
    );
    assert!(script.contains("/api/agent-codes/redeem"));
    assert!(script.contains("/api/clients/cli/linux-"));

    // Behind a proxy, the address the agent reached it on.
    let proxied = Client::new()
        .get(server.url("/agent/install.sh"))
        .header("x-forwarded-host", "vault.example")
        .header("x-forwarded-proto", "https")
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(
        proxied.contains("VAULT_URL='https://vault.example'"),
        "{proxied}"
    );
    let bad = Client::new()
        .get(server.url("/agent/install.sh"))
        .header("x-forwarded-host", "evil'$(id)")
        .send()
        .unwrap();
    assert_eq!(bad.status().as_u16(), 400);
}
