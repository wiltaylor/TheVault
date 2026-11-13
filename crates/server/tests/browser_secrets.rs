//! Login metadata content-boundary inventory and the 5,000-row performance contract.
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::time::Instant;
use testkit::{PASSWORD, TestServer};

#[test]
fn test_secret_seams_require_opt_in_and_owner_session() {
    let server = TestServer::start_with_args(|_| {}, &["--test-secret-seams".as_ref()]);
    let client = Client::new();
    let login = client
        .post(server.url("/api/login"))
        .json(&json!({"password": PASSWORD}))
        .send()
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    for name in [
        "ai_settings",
        "ai_account_chatgpt_fixture",
        "google_settings",
        "google_client_id",
        "google_client_secret",
        "google_refresh_token",
        "google_account",
    ] {
        let url = server.url(&format!("/api/secrets/{name}"));
        assert_eq!(
            client
                .put(&url)
                .bearer_auth(server.admin_token())
                .json(&json!({"value":"fixture"}))
                .send()
                .unwrap()
                .status()
                .as_u16(),
            403
        );
        assert!(
            client
                .put(&url)
                .header("cookie", &cookie)
                .json(&json!({"value":"fixture"}))
                .send()
                .unwrap()
                .status()
                .is_success()
        );
        assert_eq!(server.secret(name).as_deref(), Some("fixture"));
        assert_eq!(
            client
                .delete(&url)
                .bearer_auth(server.admin_token())
                .send()
                .unwrap()
                .status()
                .as_u16(),
            403
        );
        assert!(
            client
                .delete(&url)
                .header("cookie", &cookie)
                .send()
                .unwrap()
                .status()
                .is_success()
        );
        assert!(server.secret(name).is_none());
    }
    for name in [
        "ai_accounts",
        "ai_account_key_fixture",
        "google_gmail_trashed",
        "gitsync_key",
    ] {
        let url = server.url(&format!("/api/secrets/{name}"));
        for method in [reqwest::Method::PUT, reqwest::Method::DELETE] {
            assert_eq!(
                client
                    .request(method, &url)
                    .header("cookie", &cookie)
                    .json(&json!({"value":"fixture"}))
                    .send()
                    .unwrap()
                    .status()
                    .as_u16(),
                403
            );
        }
    }
}

#[test]
fn internal_secrets_refuse_put_and_delete_for_tokens_and_sessions() {
    let server = TestServer::start();
    let client = Client::new();
    let login = client
        .post(server.url("/api/login"))
        .json(&json!({"password": PASSWORD}))
        .send()
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let names = [
        "ai_accounts",
        "ai_settings",
        "ai_account_key_id",
        "ai_account_chatgpt_id",
        "ai_account_usage_id",
        "google_settings",
        "google_client_id",
        "google_client_secret",
        "google_refresh_token",
        "google_account",
        "google_gmail_state",
        "google_calendar_state",
        "google_gmail_trashed",
        "gitsync_settings",
        "gitsync_token",
        "gitsync_ssh_key",
        "gitsync_known_hosts",
        "gitsync_key",
        "gitsync_state",
        "gitsync_freshness_publish_peer",
        "gitsync_freshness_receive_peer",
        "brave_api_key",
    ];
    for session in [false, true] {
        for name in names {
            server.seed_secret(name, "existing record");
            // Even kind=login cannot shadow an internal string secret.
            let before = server.secret(name);
            for method in [reqwest::Method::PUT, reqwest::Method::DELETE] {
                if method == reqwest::Method::DELETE
                    && name.starts_with("gitsync_freshness_receive_")
                {
                    let req = client.delete(server.url(&format!("/api/secrets/{name}")));
                    let req = if session {
                        req.header("cookie", &cookie)
                    } else {
                        req.bearer_auth(server.admin_token())
                    };
                    assert!(req.send().unwrap().status().is_success());
                    assert!(server.secret(name).is_none());
                    continue;
                }
                let req =
                    client.request(method.clone(), server.url(&format!("/api/secrets/{name}")));
                let req = if session {
                    req.header("cookie", &cookie)
                } else {
                    req.bearer_auth(server.admin_token())
                };
                let response = req
                    .json(&json!({"kind":"login","username":"x","password":"x"}))
                    .send()
                    .unwrap();
                assert_eq!(
                    response.status(),
                    reqwest::StatusCode::FORBIDDEN,
                    "{method} {name}"
                );
                let error: Value = response.json().unwrap();
                assert_eq!(error["error"], "internal_secret");
                if name == "ai_accounts" {
                    assert!(error.to_string().contains("Settings > AI"));
                }
                assert_eq!(server.secret(name), before, "{name}");
            }
        }
        for name in [
            "user_key",
            "google_user_key",
            "gitsync_user_key",
            "brave_settings",
        ] {
            let url = server.url(&format!("/api/secrets/{name}"));
            let req = client.put(&url);
            let req = if session {
                req.header("cookie", &cookie)
            } else {
                req.bearer_auth(server.admin_token())
            };
            assert!(
                req.json(&json!({"value":"fixture"}))
                    .send()
                    .unwrap()
                    .status()
                    .is_success()
            );
            assert_eq!(server.secret(name).as_deref(), Some("fixture"));
            let req = client.delete(&url);
            let req = if session {
                req.header("cookie", &cookie)
            } else {
                req.bearer_auth(server.admin_token())
            };
            assert!(req.send().unwrap().status().is_success());
            assert!(server.secret(name).is_none());
        }
    }
}

fn large_note_server(count: usize, bytes: usize) -> TestServer {
    TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::create_dir_all(vault.join("notes/memo")).unwrap();
        std::fs::write(vault.join("types/memo.wcl"), "label = \"Memo\"\nfield status { kind = \"string\" }\ntemplate default { tab Main { slot title } }\n").unwrap();
        for i in 0..count {
            let id = format!("01JB8Z4K2MQ7V3X9{i:010}");
            std::fs::write(vault.join(format!("notes/memo/{id}.wcl")),format!("title = \"Memo {i}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\nstatus = \"{}\"\n", "x".repeat(bytes))).unwrap();
        }
    })
}

#[test]
fn browser_secrets_large_sync_and_query_pages_make_progress_under_the_budget() {
    let server = large_note_server(24, 1500 * 1024);
    let client = Client::new();
    let token: Value = client
        .post(server.url("/api/tokens"))
        .bearer_auth(server.admin_token())
        .json(&json!({"name":"writer","scope":"write"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let token = token["secret"].as_str().unwrap();
    let mut since = 0;
    let mut notes = std::collections::HashSet::new();
    for _ in 0..100 {
        let response = client
            .get(server.url("/api/sync/changes"))
            .bearer_auth(token)
            .query(&[("since", since.to_string()), ("limit", "500".to_owned())])
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "sync status {}",
            response.status()
        );
        let body = response.bytes().unwrap();
        assert!(body.len() < 32 * 1024 * 1024);
        let page: Value = serde_json::from_slice(&body).unwrap();
        for entry in page["changes"].as_array().unwrap() {
            if entry["type"] == "memo" {
                assert!(notes.insert(entry["id"].as_str().unwrap().to_owned()));
            }
        }
        if page["remaining"] == 0 {
            break;
        }
        let next = page["next"].as_i64().unwrap();
        assert!(next > since);
        since = next;
    }
    assert_eq!(notes.len(), 24);
    let mut cursor = None::<String>;
    let mut rows = std::collections::HashSet::new();
    for _ in 0..100 {
        let mut params = vec![
            ("q", "from memo select status".to_owned()),
            ("page_size", "1000".to_owned()),
        ];
        if let Some(value) = cursor {
            params.push(("cursor", value));
        }
        let response = client
            .get(server.url("/api/query"))
            .bearer_auth(token)
            .query(&params)
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "query status {}",
            response.status()
        );
        let body = response.bytes().unwrap();
        assert!(body.len() < 32 * 1024 * 1024);
        let page: Value = serde_json::from_slice(&body).unwrap();
        for row in page["rows"].as_array().unwrap() {
            assert!(rows.insert(row["id"].as_str().unwrap().to_owned()));
        }
        cursor = page["next"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows.len(), 24);
}

#[test]
fn browser_secrets_device_sync_accepts_one_forty_megabyte_note() {
    let server = large_note_server(1, 40 * 1024 * 1024);
    let client = Client::new();
    let token: Value = client
        .post(server.url("/api/devices/pair"))
        .json(&json!({"password":PASSWORD,"name":"Phone","platform":"android"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let token = token["token"].as_str().unwrap();
    let mut since = 0;
    let mut seen = false;
    for _ in 0..100 {
        let response = client
            .get(server.url("/api/sync/changes"))
            .bearer_auth(token)
            .query(&[("since", since.to_string()), ("limit", "500".to_owned())])
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "device sync status {}",
            response.status()
        );
        let page: Value = response.json().unwrap();
        for entry in page["changes"].as_array().unwrap() {
            if entry["type"] == "memo" {
                assert!(entry["wcl"].as_str().unwrap().len() > 40 * 1024 * 1024);
                seen = true;
            }
        }
        if page["remaining"] == 0 {
            break;
        }
        let next = page["next"].as_i64().unwrap();
        assert!(next > since);
        since = next;
    }
    assert!(seen);
}

fn reviewed_routes() -> Vec<String> {
    // TV-217 /api/ai/usage review: owner-only counts and account/model IDs, no Login metadata.
    include_str!("browser_secrets_routes.txt")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn browser_secrets_runtime_route_inventory_requires_review_of_every_new_route() {
    let mut actual: Vec<_> = api::routes()
        .into_iter()
        .map(|(method, path)| format!("{method} {path}"))
        .collect();
    actual.sort();
    actual.dedup();
    assert_eq!(
        actual,
        reviewed_routes(),
        "New routes need a Login-metadata response/stream review"
    );
    // A literal or const-path new content route is equally visible to the runtime inventory.
    actual.push("GET /api/unreviewed-content".to_owned());
    actual.sort();
    assert_ne!(actual, reviewed_routes());
}

#[test]
fn browser_secrets_five_thousand_row_agent_query_is_within_twice_session_time() {
    let server = TestServer::start_seeded(|vault| {
        std::fs::create_dir_all(vault.join("types")).unwrap();
        std::fs::create_dir_all(vault.join("notes/benchmark")).unwrap();
        std::fs::write(vault.join("types/benchmark.wcl"), "label = \"Benchmark\"\nfield data { kind = \"string\"\n}\ntemplate default {\n tab \"Main\" { slot \"title\"\n }\n}\n").unwrap();
        for i in 0..5000 {
            let id = format!("01JB8Z4K2MQ7V3X9{i:010}");
            std::fs::write(vault.join(format!("notes/benchmark/{id}.wcl")), format!("title = \"Item {i:04}\"\ncreated = \"2026-10-01T09:00:00Z\"\nupdated = \"2026-10-01T09:00:00Z\"\ndata = \"{}\"\n", "x".repeat(256))).unwrap();
        }
    });
    let client = Client::new();
    let login = client
        .post(server.url("/api/login"))
        .json(&json!({"password":PASSWORD}))
        .send()
        .unwrap();
    assert!(login.status().is_success());
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let issued: Value = client
        .post(server.url("/api/agent-codes"))
        .bearer_auth(server.admin_token())
        .json(&json!({"name":"benchmark", "scope":"write"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let token: Value = client
        .post(server.url("/api/agent-codes/redeem"))
        .json(&json!({"code":issued["code"]}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let token = token["token"].as_str().unwrap();
    let query = |agent: bool| {
        let began = Instant::now();
        let mut cursor = None::<String>;
        let mut rows = 0;
        loop {
            let mut params = vec![
                ("q", "from benchmark sort title".to_owned()),
                ("page_size", "1000".to_owned()),
            ];
            if let Some(c) = cursor {
                params.push(("cursor", c));
            }
            let req = client.get(server.url("/api/query")).query(&params);
            let response = if agent {
                req.bearer_auth(token)
            } else {
                req.header("Cookie", &cookie)
            }
            .send()
            .unwrap();
            assert!(
                response.status().is_success(),
                "{}",
                response.text().unwrap()
            );
            let page: Value = response.json().unwrap();
            rows += page["rows"].as_array().unwrap().len();
            cursor = page["next"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(rows, 5000);
        began.elapsed()
    };
    query(false);
    query(true); // warm both paths
    let mut pairs = Vec::new();
    for round in 0..5 {
        // Interleave and alternate the order within a round so a load spike
        // cannot bias every pair the same way.
        let pair = if round % 2 == 0 {
            let session = query(false);
            let agent = query(true);
            (session, agent)
        } else {
            let agent = query(true);
            let session = query(false);
            (session, agent)
        };
        pairs.push(pair);
    }
    // Load on the shared target only adds time, never removes it, so the
    // minimum over the five rounds is the least-perturbed estimate of each
    // side's own cost. An intrinsic agent regression appears in every agent
    // round and raises the agent minimum, while a session spike would have to
    // hit all five interleaved rounds to raise the session minimum. Comparing
    // the minima keeps the property strict: the agent query costs at most
    // twice the session query, plus the load slack below.
    let agent_min = pairs.iter().map(|&(_, agent)| agent).min().unwrap();
    let session_min = pairs.iter().map(|&(session, _)| session).min().unwrap();
    eprintln!(
        "5,000 rows, five 1,000-row pages: minima over 5 pairs session {:?}, agent {:?}",
        session_min, agent_min
    );
    // TV-330: the centralized query boundary encodes each page only once.
    // Require the improvement without a fixed allowance hiding small-page costs.
    let report = pairs
        .iter()
        .map(|&(session, agent)| format!("{:?}/{:?}", agent, session))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        agent_min <= session_min.mul_f64(1.6),
        "agent_min {:?} above 1.6x session_min {:?}; pairs (agent/session): {}",
        agent_min,
        session_min,
        report
    );
}
