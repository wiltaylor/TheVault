//! S1: password login and sessions, scoped API tokens, and the scope every endpoint needs (D11).

use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, SET_COOKIE};
use serde_json::{Value, json};
use testkit::TestServer;

const TASK_TYPE: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate"
  }
}
"#;

const TASK_TYPE_WITH_OWNER: &str = r#"label = "Task"

field estimate {
  kind = "int"
}

field owner {
  kind = "string"
}

template default {
  tab "Main" {
    slot "title"
    slot "estimate"
    slot "owner"
  }
}
"#;

const WCL: &str = "text/plain; charset=utf-8";

fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/task.wcl"), TASK_TYPE).unwrap();
}

/// How a request authenticates.
#[derive(Clone, Copy)]
enum Auth<'a> {
    None,
    Bearer(&'a str),
    Cookie(&'a str),
}

struct Api {
    server: TestServer,
    client: Client,
}

impl Api {
    fn start() -> Api {
        Api {
            server: TestServer::start_seeded(seed),
            client: Client::new(),
        }
    }

    fn admin(&self) -> Auth<'_> {
        Auth::Bearer(self.server.admin_token())
    }

    fn req(&self, method: &str, path: &str, auth: Auth<'_>) -> RequestBuilder {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let req = self.client.request(method, self.server.url(path));
        match auth {
            Auth::None => req,
            Auth::Bearer(t) => req.header(AUTHORIZATION, format!("Bearer {t}")),
            Auth::Cookie(c) => req.header(COOKIE, c),
        }
    }

    /// Create a token with the admin token and return its secret.
    fn token(&self, name: &str, scope: &str) -> String {
        let res = self
            .req("POST", "/api/tokens", self.admin())
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        let body = expect(res, 201);
        assert_eq!(body["name"], name);
        assert_eq!(body["scope"], scope);
        s(&body["secret"]).to_owned()
    }

    fn create_note(&self, auth: Auth<'_>) -> Response {
        self.req("POST", "/api/notes?type=task", auth)
            .header(CONTENT_TYPE, WCL)
            .body("title = \"Write the auth ticket\"\nestimate = 3\n")
            .send()
            .unwrap()
    }

    fn put_type(&self, auth: Auth<'_>, wcl: &str) -> Response {
        self.put_named_type(auth, "task", wcl)
    }

    fn put_named_type(&self, auth: Auth<'_>, name: &str, wcl: &str) -> Response {
        self.req("PUT", &format!("/api/types/{name}"), auth)
            .header(CONTENT_TYPE, WCL)
            .body(wcl.to_owned())
            .send()
            .unwrap()
    }

    fn login(&self, password: &str) -> Response {
        self.req("POST", "/api/login", Auth::None)
            .json(&json!({ "password": password }))
            .send()
            .unwrap()
    }
}

/// Assert the status and return the JSON body, or `Null` for an empty one.
fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "unexpected status, body: {text}");
    if text.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: body is not JSON: {text}"))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("{v} is not a string"))
}

/// The `name=value` part of a `Set-Cookie` header.
fn cookie_pair(res: &Response) -> String {
    let set = res
        .headers()
        .get(SET_COOKIE)
        .expect("a Set-Cookie header")
        .to_str()
        .unwrap();
    set.split(';').next().unwrap().trim().to_owned()
}

#[test]
fn without_credentials_every_non_public_endpoint_is_401() {
    let api = Api::start();
    let id = s(&expect(api.create_note(api.admin()), 201)["id"]).to_owned();
    let note = format!("/api/notes/{id}");
    let endpoints = [
        ("GET", "/api/notes".to_owned()),
        ("POST", "/api/notes?type=task".to_owned()),
        ("GET", note.clone()),
        ("PUT", note.clone()),
        ("DELETE", note.clone()),
        ("GET", format!("{note}/history")),
        ("POST", format!("{note}/restore")),
        ("GET", "/api/trash".to_owned()),
        ("POST", format!("/api/trash/{id}/restore")),
        ("GET", "/api/types/task".to_owned()),
        ("PUT", "/api/types/task".to_owned()),
        ("GET", "/api/tokens".to_owned()),
        ("POST", "/api/tokens".to_owned()),
        ("DELETE", "/api/tokens/admin".to_owned()),
        ("POST", "/api/logout".to_owned()),
        ("GET", "/api/no-such-endpoint".to_owned()),
    ];
    for (method, path) in &endpoints {
        for auth in [
            Auth::None,
            Auth::Bearer("tv_not-a-real-token"),
            Auth::Cookie("tv_session=not-a-real-session"),
        ] {
            let res = api.req(method, path, auth).send().unwrap();
            let status = res.status().as_u16();
            let body = res.text().unwrap();
            assert_eq!(status, 401, "{method} {path} answered {status}: {body}");
            assert!(body.contains("unauthorized"), "{method} {path}: {body}");
        }
    }

    // Nothing changed.
    let body = expect(api.req("GET", &note, api.admin()).send().unwrap(), 200);
    assert_eq!(body["id"], id.as_str());

    // Health and login are public.
    expect(
        api.req("GET", "/api/health", Auth::None).send().unwrap(),
        200,
    );
    expect(api.login("wrong"), 401);
}

#[test]
fn a_read_token_reads_and_gets_403_on_write() {
    let api = Api::start();
    let created = expect(api.create_note(api.admin()), 201);
    let id = s(&created["id"]);
    let version = s(&created["version"]);
    let read = api.token("reader", "read");
    let auth = Auth::Bearer(&read);

    let body = expect(
        api.req("GET", &format!("/api/notes/{id}"), auth)
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(body["id"], id);
    expect(api.req("GET", "/api/notes", auth).send().unwrap(), 200);
    expect(
        api.req("GET", &format!("/api/notes/{id}/history"), auth)
            .send()
            .unwrap(),
        200,
    );
    expect(api.req("GET", "/api/types/task", auth).send().unwrap(), 200);

    let body = expect(api.create_note(auth), 403);
    assert_eq!(body["error"], "forbidden");
    let res = api
        .req("PUT", &format!("/api/notes/{id}"), auth)
        .header(CONTENT_TYPE, WCL)
        .header("If-Match", format!("\"{version}\""))
        .body("title = \"Changed\"\n")
        .send()
        .unwrap();
    expect(res, 403);
    let res = api
        .req("DELETE", &format!("/api/notes/{id}"), auth)
        .header("If-Match", format!("\"{version}\""))
        .send()
        .unwrap();
    expect(res, 403);
    expect(api.put_type(auth, TASK_TYPE_WITH_OWNER), 403);
    expect(api.req("GET", "/api/tokens", auth).send().unwrap(), 403);

    // The note is untouched.
    let body = expect(
        api.req("GET", &format!("/api/notes/{id}"), auth)
            .send()
            .unwrap(),
        200,
    );
    assert_eq!(body["version"], version);
}

#[test]
fn a_write_token_writes_notes_and_gets_403_on_changing_a_type() {
    let api = Api::start();
    let write = api.token("agent", "write");
    let auth = Auth::Bearer(&write);

    let created = expect(api.create_note(auth), 201);
    let id = s(&created["id"]);
    let version = s(&created["version"]);
    let res = api
        .req("PUT", &format!("/api/notes/{id}"), auth)
        .header(CONTENT_TYPE, WCL)
        .header("If-Match", format!("\"{version}\""))
        .body("title = \"Changed\"\nestimate = 5\n")
        .send()
        .unwrap();
    let updated = expect(res, 200);

    // The write is recorded against the token's name (D13).
    let history = expect(
        api.req("GET", &format!("/api/notes/{id}/history"), auth)
            .send()
            .unwrap(),
        200,
    );
    let authors: Vec<&Value> = history["versions"]
        .as_array()
        .unwrap_or_else(|| panic!("no versions in {history}"))
        .iter()
        .map(|v| &v["author"])
        .collect();
    assert!(!authors.is_empty());
    for author in authors {
        assert_eq!(author, &json!({ "kind": "token", "name": "agent" }));
    }

    let res = api
        .req("DELETE", &format!("/api/notes/{id}"), auth)
        .header("If-Match", format!("\"{}\"", s(&updated["version"])))
        .send()
        .unwrap();
    assert!(res.status().is_success(), "delete: {}", res.status());

    let body = expect(api.put_type(auth, TASK_TYPE_WITH_OWNER), 403);
    assert_eq!(body["error"], "forbidden");
    let on_disk = std::fs::read_to_string(api.server.vault_path().join("types/task.wcl")).unwrap();
    assert_eq!(on_disk, TASK_TYPE);

    // Tokens are admin-only too.
    expect(api.req("GET", "/api/tokens", auth).send().unwrap(), 403);
    let res = api
        .req("POST", "/api/tokens", auth)
        .json(&json!({ "name": "escalate", "scope": "admin" }))
        .send()
        .unwrap();
    expect(res, 403);
}

#[test]
fn an_admin_token_can_change_a_type() {
    let api = Api::start();
    let admin = api.token("root", "admin");
    let auth = Auth::Bearer(&admin);

    let body = expect(api.put_type(auth, TASK_TYPE_WITH_OWNER), 200);
    assert_eq!(body["name"], "task");
    let on_disk = std::fs::read_to_string(api.server.vault_path().join("types/task.wcl")).unwrap();
    assert_eq!(s(&body["wcl"]), on_disk);
    assert!(on_disk.contains("field owner"), "{on_disk}");

    // The new field is live: a note may set it.
    let res = api
        .req("POST", "/api/notes?type=task", auth)
        .header(CONTENT_TYPE, WCL)
        .body("title = \"Owned\"\nowner = \"wil\"\n")
        .send()
        .unwrap();
    expect(res, 201);

    let res = api
        .req("GET", "/api/types/task", auth)
        .header("Accept", WCL)
        .send()
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.text().unwrap(), on_disk);

    // Changing a type that now has notes needs a planned schema change (ticket 10, D6).
    let body = expect(api.put_type(auth, TASK_TYPE), 409);
    assert_eq!(body["error"], "type_in_use", "{body}");

    // A type that does not validate is refused and the file is left alone.
    let body = expect(
        api.put_named_type(auth, "broken", "field broken {\n  kind = \"nope\"\n}\n"),
        400,
    );
    assert_eq!(body["error"], "invalid_type", "{body}");
    assert!(!api.server.vault_path().join("types/broken.wcl").exists());
    let body = expect(api.put_named_type(auth, "Bad-Name", TASK_TYPE), 400);
    assert_eq!(body["error"], "invalid_type", "{body}");
}

#[test]
fn a_revoked_token_gets_401() {
    let api = Api::start();
    let read = api.token("short-lived", "read");
    expect(
        api.req("GET", "/api/notes", Auth::Bearer(&read))
            .send()
            .unwrap(),
        200,
    );

    let res = api
        .req("DELETE", "/api/tokens/short-lived", api.admin())
        .send()
        .unwrap();
    expect(res, 204);

    let body = expect(
        api.req("GET", "/api/notes", Auth::Bearer(&read))
            .send()
            .unwrap(),
        401,
    );
    assert_eq!(body["error"], "unauthorized");

    let res = api
        .req("DELETE", "/api/tokens/short-lived", api.admin())
        .send()
        .unwrap();
    expect(res, 404);
}

#[test]
fn token_secrets_are_shown_once_and_listed_without_them() {
    let api = Api::start();
    let secret = api.token("cli", "write");
    assert!(secret.starts_with("tv_"), "{secret}");

    let list = expect(
        api.req("GET", "/api/tokens", api.admin()).send().unwrap(),
        200,
    );
    let tokens = list["tokens"].as_array().expect("a tokens array");
    let cli = tokens
        .iter()
        .find(|t| t["name"] == "cli")
        .unwrap_or_else(|| panic!("no cli token in {list}"));
    assert_eq!(cli["scope"], "write");
    assert!(cli.get("secret").is_none(), "{cli}");
    assert!(!list.to_string().contains(&secret));

    let res = api
        .req("POST", "/api/tokens", api.admin())
        .json(&json!({ "name": "cli", "scope": "read" }))
        .send()
        .unwrap();
    expect(res, 409);
    let res = api
        .req("POST", "/api/tokens", api.admin())
        .json(&json!({ "name": "other", "scope": "superuser" }))
        .send()
        .unwrap();
    expect(res, 400);
}

#[test]
fn login_sets_a_session_cookie_that_authorises_reads() {
    let api = Api::start();

    let res = api.login("not the password");
    assert!(res.headers().get(SET_COOKIE).is_none());
    let body = expect(res, 401);
    assert_eq!(body["error"], "unauthorized");
    expect(api.login(""), 401);
    let res = api
        .req("POST", "/api/login", Auth::None)
        .json(&json!({}))
        .send()
        .unwrap();
    assert!(res.status().is_client_error(), "{}", res.status());

    let res = api.login(testkit::PASSWORD);
    let cookie = cookie_pair(&res);
    let set = res.headers()[SET_COOKIE].to_str().unwrap().to_owned();
    assert!(set.contains("HttpOnly"), "{set}");
    expect(res, 200);
    let auth = Auth::Cookie(&cookie);

    expect(api.req("GET", "/api/notes", auth).send().unwrap(), 200);
    // The browser session is the owner: it may write notes and change types.
    let created = expect(api.create_note(auth), 201);
    let history = expect(
        api.req(
            "GET",
            &format!("/api/notes/{}/history", s(&created["id"])),
            auth,
        )
        .send()
        .unwrap(),
        200,
    );
    assert_eq!(history["versions"][0]["author"]["kind"], "session");
    expect(api.put_named_type(auth, "project", TASK_TYPE), 200);

    // Logging out ends the session.
    expect(api.req("POST", "/api/logout", auth).send().unwrap(), 204);
    expect(api.req("GET", "/api/notes", auth).send().unwrap(), 401);
}

/// The clock `--test-clock` starts the session expiry tests at.
const CLOCK_START: &str = "2026-03-01T09:00:00Z";

const THIRTY_DAYS: u64 = 30 * 24 * 60 * 60;

impl Api {
    fn start_with_clock() -> Api {
        Api {
            server: TestServer::start_with_args(
                seed,
                &[
                    std::ffi::OsStr::new("--test-clock"),
                    std::ffi::OsStr::new(CLOCK_START),
                ],
            ),
            client: Client::new(),
        }
    }

    fn advance(&self, by: &str) {
        let res = self
            .req("POST", "/api/test/clock", self.admin())
            .json(&json!({ "advance": by }))
            .send()
            .unwrap();
        expect(res, 200);
    }

    /// Log in and return the `name=value` of the session cookie.
    fn session(&self) -> String {
        let res = self.login(testkit::PASSWORD);
        let cookie = cookie_pair(&res);
        assert_eq!(max_age(&res), THIRTY_DAYS);
        expect(res, 200);
        cookie
    }

    fn notes(&self, cookie: &str) -> Response {
        self.req("GET", "/api/notes", Auth::Cookie(cookie))
            .send()
            .unwrap()
    }

    fn state_db(&self) -> std::path::PathBuf {
        self.server.vault_path().join(".vault/state.db")
    }
}

/// The `Max-Age` of a `Set-Cookie` header, in seconds.
fn max_age(res: &Response) -> u64 {
    let set = res.headers()[SET_COOKIE].to_str().unwrap();
    set.split(';')
        .find_map(|part| part.trim().strip_prefix("Max-Age="))
        .unwrap_or_else(|| panic!("no Max-Age in {set}"))
        .parse()
        .unwrap()
}

#[test]
fn a_session_survives_a_server_restart_until_logout() {
    let mut api = Api::start();
    let cookie = api.session();
    expect(api.notes(&cookie), 200);

    api.server.restart();
    expect(api.notes(&cookie), 200);

    expect(
        api.req("POST", "/api/logout", Auth::Cookie(&cookie))
            .send()
            .unwrap(),
        204,
    );
    expect(api.notes(&cookie), 401);
    api.server.restart();
    expect(api.notes(&cookie), 401);
}

#[test]
fn state_db_never_holds_the_session_cookie_value() {
    let api = Api::start();
    let cookie = api.session();
    expect(api.notes(&cookie), 200);
    let id = cookie.strip_prefix("tv_session=").unwrap().as_bytes();
    let dir = api.state_db().parent().unwrap().to_owned();
    let mut files = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("state.db")
        {
            continue;
        }
        files += 1;
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.windows(id.len()).any(|w| w == id),
            "{} holds the session id",
            path.display()
        );
    }
    assert!(files > 0);
}

#[test]
fn a_session_in_use_renews_after_an_hour_and_an_idle_one_expires() {
    let api = Api::start_with_clock();
    let cookie = api.session();
    let idle = api.session();

    // Within the hour the cookie is not sent again.
    api.advance("PT59M");
    let res = api.notes(&cookie);
    assert!(res.headers().get(SET_COOKIE).is_none());
    expect(res, 200);

    // Past the hour it is, with a fresh 30 days.
    api.advance("PT2M");
    let res = api.notes(&cookie);
    assert_eq!(cookie_pair(&res), cookie);
    assert_eq!(max_age(&res), THIRTY_DAYS);
    expect(res, 200);
    let res = api.notes(&cookie);
    assert!(res.headers().get(SET_COOKIE).is_none());
    expect(res, 200);

    // 30 days after login the idle session is gone; the renewed one still works.
    api.advance("P29DT23H");
    expect(api.notes(&idle), 401);
    expect(api.notes(&cookie), 200);
}

#[test]
fn changing_the_password_logs_every_browser_out() {
    let api = Api::start();
    let first = api.session();
    let second = api.session();
    expect(api.notes(&first), 200);

    // What `vault passwd` does, against the running server's state.db.
    state::State::open(&api.state_db(), None)
        .unwrap()
        .set_password("a new password")
        .unwrap();

    expect(api.notes(&first), 401);
    expect(api.notes(&second), 401);
    expect(api.login(testkit::PASSWORD), 401);
    let fresh = {
        let res = api.login("a new password");
        let cookie = cookie_pair(&res);
        expect(res, 200);
        cookie
    };
    expect(api.notes(&fresh), 200);
}
