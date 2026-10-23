//! S1: the browser box's desktop through the server's `/browser/` proxy, for a password login
//! only, and its status for Settings > Integrations (browser B4).

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use serde_json::{Value, json};
use testkit::{FakeNoVnc, NOVNC_PAGE, TestServer, WsClient};

struct Api {
    server: TestServer,
    client: Client,
    cookie: String,
}

impl Api {
    fn start(env: &[(&str, &str)]) -> Api {
        let server = TestServer::start_with_env(|_| {}, env);
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let res = client
            .post(server.url("/api/login"))
            .json(&json!({ "password": testkit::PASSWORD }))
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let cookie = res.headers()[SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_owned();
        Api {
            server,
            client,
            cookie,
        }
    }

    fn with_box(fake: &FakeNoVnc) -> Api {
        Api::start(&[
            ("VAULT_BROWSER_URL", &fake.base_url()),
            ("VAULT_BROWSER_DESKTOP_URL", &fake.base_url()),
        ])
    }

    fn session(&self, path: &str) -> RequestBuilder {
        self.client
            .get(self.server.url(path))
            .header(COOKIE, &self.cookie)
    }

    fn bearer(&self, path: &str, token: &str) -> RequestBuilder {
        self.client
            .get(self.server.url(path))
            .header(AUTHORIZATION, format!("Bearer {token}"))
    }

    /// A token of `scope`, made with the admin token.
    fn token(&self, name: &str, scope: &str) -> String {
        let res = self
            .client
            .post(self.server.url("/api/tokens"))
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.server.admin_token()),
            )
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 201);
        res.json::<Value>().unwrap()["secret"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

#[test]
fn a_session_gets_novnc_files_and_its_websocket() {
    let fake = FakeNoVnc::start();
    let api = Api::with_box(&fake);

    let res = api.session("/browser/vnc.html").send().unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.headers()["content-type"], "text/html");
    assert_eq!(res.text().unwrap(), NOVNC_PAGE);
    let seen = fake.requests();
    let got = seen.iter().find(|r| r.route() == "/vnc.html").unwrap();
    assert_eq!(
        got.header("cookie"),
        None,
        "the session cookie stays with the server"
    );
    assert_eq!(got.header("authorization"), None);

    let res = api.session("/browser/missing.js").send().unwrap();
    assert_eq!(res.status().as_u16(), 404);

    let res = api.session("/browser/").send().unwrap();
    assert_eq!(res.status().as_u16(), 303);
    assert_eq!(
        res.headers()["location"],
        "/browser/vnc.html?autoconnect=1&resize=remote&path=browser/websockify"
    );

    let mut ws = WsClient::connect(
        api.server.addr(),
        "/browser/websockify",
        &[("Cookie", &api.cookie)],
    )
    .unwrap();
    assert_eq!(ws.status, 101);
    assert_eq!(
        ws.header("sec-websocket-accept"),
        Some(testkit::fake_novnc::ws_accept(testkit::fake_novnc::WS_KEY).as_str())
    );
    assert_eq!(ws.header("sec-websocket-protocol"), Some("binary"));
    ws.send_binary(b"RFB 003.008\n").unwrap();
    assert_eq!(ws.read_frame().unwrap(), (2, b"RFB 003.008\n".to_vec()));
    let big = vec![7u8; 70_000];
    ws.send_binary(&big).unwrap();
    assert_eq!(ws.read_frame().unwrap(), (2, big));
    ws.send_text("hello").unwrap();
    assert_eq!(ws.read_frame().unwrap(), (1, b"hello".to_vec()));
    assert_eq!(ws.close().unwrap().0, 8);
    let up = fake
        .requests()
        .into_iter()
        .find(|r| r.route() == "/websockify")
        .unwrap();
    assert_eq!(up.header("cookie"), None);
}

#[test]
fn every_token_scope_is_refused_human_only() {
    let fake = FakeNoVnc::start();
    let api = Api::with_box(&fake);
    let tokens = [
        ("read", api.token("reader", "read")),
        ("write", api.token("writer", "write")),
        ("admin", api.server.admin_token().to_owned()),
    ];
    for (scope, token) in &tokens {
        for path in ["/browser/vnc.html", "/browser/", "/browser"] {
            let res = api.bearer(path, token).send().unwrap();
            assert_eq!(res.status().as_u16(), 403, "{scope} {path}");
            let body: Value = res.json().unwrap();
            assert_eq!(body["error"], "human_only", "{scope} {path}: {body}");
            assert_eq!(
                body["message"], "Only the signed-in user can open the browser box's desktop.",
                "{body}"
            );
        }
        let auth = format!("Bearer {token}");
        let ws = WsClient::connect(
            api.server.addr(),
            "/browser/websockify",
            &[("Authorization", &auth)],
        )
        .unwrap();
        assert_eq!(ws.status, 403, "{scope} websocket");
        // A token with the session cookie is still the token's request.
        let ws = WsClient::connect(
            api.server.addr(),
            "/browser/websockify",
            &[("Authorization", &auth), ("Cookie", &api.cookie)],
        )
        .unwrap();
        assert_eq!(ws.status, 403, "{scope} websocket with a cookie");
    }
    let ws = WsClient::connect(api.server.addr(), "/browser/websockify", &[]).unwrap();
    assert_eq!(ws.status, 401);
    let res = api
        .client
        .get(api.server.url("/browser/vnc.html"))
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);
    assert!(
        fake.requests().iter().all(|r| r.route() == "/health"),
        "nothing reached the desktop: {:?}",
        fake.requests()
    );
}

#[test]
fn the_desktop_is_absent_without_the_box() {
    let api = Api::start(&[("VAULT_BROWSER_URL", ""), ("VAULT_BROWSER_DESKTOP_URL", "")]);
    for path in ["/browser/vnc.html", "/browser/websockify"] {
        let res = api.session(path).send().unwrap();
        assert_eq!(res.status().as_u16(), 404, "{path}");
    }
    let res = api
        .bearer("/api/integrations/browser", api.server.admin_token())
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().unwrap();
    assert_eq!(
        body,
        json!({
            "configured": false, "reachable": false, "version": null,
            "playwright_cli": null, "display": null, "error": null,
        })
    );
}

#[test]
fn the_status_reads_the_exec_service_health() {
    let fake = FakeNoVnc::start();
    let api = Api::with_box(&fake);
    let res = api.session("/api/integrations/browser").send().unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let body: Value = res.json().unwrap();
    assert_eq!(body["configured"], true, "{body}");
    assert_eq!(body["reachable"], true, "{body}");
    assert_eq!(body["version"], "0.0.0-fake");
    assert_eq!(body["playwright_cli"], "0.1.22");
    assert_eq!(body["display"], ":1");
    assert_eq!(body["error"], Value::Null);

    // Admin only.
    let reader = api.token("reader", "read");
    let res = api
        .bearer("/api/integrations/browser", &reader)
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 403);
}

#[test]
fn the_status_says_when_the_box_does_not_answer() {
    // A port nothing listens on: bind one, then let it go.
    let gone = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let api = Api::start(&[
        ("VAULT_BROWSER_URL", &gone),
        ("VAULT_BROWSER_DESKTOP_URL", &gone),
    ]);
    let body: Value = api
        .session("/api/integrations/browser")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(body["configured"], true, "{body}");
    assert_eq!(body["reachable"], false, "{body}");
    assert!(body["error"].as_str().is_some(), "{body}");
    let res = api.session("/browser/vnc.html").send().unwrap();
    assert_eq!(res.status().as_u16(), 502);
    let ws = WsClient::connect(
        api.server.addr(),
        "/browser/websockify",
        &[("Cookie", &api.cookie)],
    )
    .unwrap();
    assert_eq!(ws.status, 502);
}
