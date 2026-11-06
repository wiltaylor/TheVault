//! `POST /api/inbox/mark-processed` (TV-30): the inbox's one-time catch-up over HTTP. The rules
//! themselves are tested in the writer's `inbox` tests.

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};
use testkit::TestServer;

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn post(server: &TestServer, token: &str, path: &str, body: Value) -> Response {
    Client::new()
        .post(server.url(path))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .json(&body)
        .send()
        .unwrap()
}

fn create(server: &TestServer, ty: &str, fields: Value) -> String {
    let made = expect(
        post(
            server,
            &server.admin_token(),
            &format!("/api/notes?type={ty}"),
            json!({ "note": { "fields": fields } }),
        ),
        StatusCode::CREATED,
    );
    made["id"].as_str().unwrap().to_owned()
}

const MARK: &str = "/api/inbox/mark-processed";

#[test]
fn a_dry_run_counts_and_a_run_marks_the_old_mail_messages_and_calls() {
    let server = TestServer::start();
    let admin = server.admin_token();
    create(&server, "email", json!({ "title": "Old", "sent": "2026-09-01T10:00:00Z" }));
    create(&server, "email", json!({ "title": "New", "sent": "2026-11-02T10:00:00Z" }));
    create(
        &server,
        "message",
        json!({ "title": "Hi", "direction": "in", "sent": "2026-10-01T10:00:00Z", "device_ref": "sms:1" }),
    );
    create(
        &server,
        "call",
        json!({ "title": "Missed", "direction": "missed", "started": "2026-10-02T10:00:00Z", "device_ref": "call:1" }),
    );
    create(&server, "fleeting", json!({ "title": "A thought" }));

    let body = json!({ "before": "2026-11-01T00:00:00Z", "dry_run": true });
    let dry = expect(post(&server, &admin, MARK, body), StatusCode::OK);
    assert_eq!(
        dry,
        json!({
            "dry_run": true,
            "matched": { "email": 1, "message": 1, "call": 1 },
            "total": 3,
            "changed": 0,
            "chunks": 0,
        })
    );

    let body = json!({ "before": "2026-11-01T00:00:00Z" });
    let done = expect(post(&server, &admin, MARK, body.clone()), StatusCode::OK);
    assert_eq!(done["changed"], 3);
    assert_eq!(done["chunks"], 1);
    let again = expect(post(&server, &admin, MARK, body), StatusCode::OK);
    assert_eq!(again["total"], 0);
    assert_eq!(again["changed"], 0);

    let left = expect(
        post(&server, &admin, "/api/query", json!({ "query": "from fleeting where !processed" })),
        StatusCode::OK,
    );
    assert_eq!(left["total"], 1, "fleeting notes are not touched");
}

#[test]
fn a_date_that_does_not_read_is_refused_and_a_write_token_may_not_run_it() {
    let server = TestServer::start();
    let admin = server.admin_token();
    let res = post(&server, &admin, MARK, json!({ "before": "last week" }));
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let token = expect(
        post(&server, &admin, "/api/tokens", json!({ "name": "agent", "scope": "write" })),
        StatusCode::CREATED,
    );
    let res = post(
        &server,
        token["secret"].as_str().unwrap(),
        MARK,
        json!({ "before": "2026-11-01T00:00:00Z", "dry_run": true }),
    );
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
