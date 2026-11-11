//! Login metadata content-boundary inventory and the 5,000-row performance contract.
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::time::Instant;
use testkit::{PASSWORD, TestServer};

fn reviewed_routes() -> Vec<String> {
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
    let mut session = Vec::new();
    let mut agent = Vec::new();
    for round in 0..3 {
        if round % 2 == 0 {
            session.push(query(false));
            agent.push(query(true));
        } else {
            agent.push(query(true));
            session.push(query(false));
        }
    }
    session.sort();
    agent.sort();
    eprintln!(
        "5,000 rows, five 1,000-row pages: session median {:?}, agent median {:?}",
        session[1], agent[1]
    );
    assert!(
        agent[1] <= session[1] * 2,
        "agent {:?}, session {:?}",
        agent[1],
        session[1]
    );
}
