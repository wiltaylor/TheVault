//! S1: emptying the note trash, which only the signed-in user may do (ai D21, D13).

use std::path::Path;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
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

/// How a request authenticates.
#[derive(Clone)]
enum Auth {
    Bearer(String),
    Cookie(String),
}

struct Api {
    server: TestServer,
    client: Client,
    session: Auth,
    admin: Auth,
}

impl Api {
    fn start() -> Api {
        let server = TestServer::start_seeded(seed);
        let client = Client::new();
        let res = client
            .post(server.url("/api/login"))
            .json(&json!({ "password": testkit::PASSWORD }))
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let cookie = res
            .headers()
            .get(SET_COOKIE)
            .expect("a Set-Cookie header")
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_owned();
        let admin = Auth::Bearer(server.admin_token().to_owned());
        Api {
            server,
            client,
            session: Auth::Cookie(cookie),
            admin,
        }
    }

    fn req(&self, method: &str, path: &str, auth: &Auth) -> RequestBuilder {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let req = self.client.request(method, self.server.url(path));
        match auth {
            Auth::Bearer(t) => req.header(AUTHORIZATION, format!("Bearer {t}")),
            Auth::Cookie(c) => req.header(COOKIE, c),
        }
    }

    /// A token of `scope`, made with the admin token.
    fn token(&self, name: &str, scope: &str) -> Auth {
        let res = self
            .req("POST", "/api/tokens", &self.admin)
            .json(&json!({ "name": name, "scope": scope }))
            .send()
            .unwrap();
        Auth::Bearer(s(&expect(res, 201)["secret"]).to_owned())
    }

    fn upload(&self, name: &str) -> String {
        let res = self
            .req("POST", &format!("/api/assets?filename={name}"), &self.admin)
            .body(name.as_bytes().to_vec())
            .send()
            .unwrap();
        s(&expect(res, 201)["id"]).to_owned()
    }

    /// Create a doc, change it once so it has history, delete it, and return its id.
    fn trashed(&self, title: &str, file: Option<&str>) -> String {
        let mut fields = json!({ "title": title });
        if let Some(f) = file {
            fields["file"] = json!(f);
        }
        let created = self.create(fields.clone());
        let id = s(&created["id"]).to_owned();
        fields["title"] = json!(format!("{title}, edited"));
        let res = self
            .req("PUT", &format!("/api/notes/{id}"), &self.admin)
            .json(&json!({ "base_version": created["version"], "note": { "fields": fields } }))
            .send()
            .unwrap();
        let version = s(&expect(res, 200)["version"]).to_owned();
        let res = self
            .req(
                "DELETE",
                &format!("/api/notes/{id}?base_version={version}"),
                &self.admin,
            )
            .send()
            .unwrap();
        expect(res, 200);
        id
    }

    fn create(&self, fields: Value) -> Value {
        let res = self
            .req("POST", "/api/notes?type=doc", &self.admin)
            .json(&json!({ "note": { "fields": fields } }))
            .send()
            .unwrap();
        expect(res, 201)
    }

    fn trash_ids(&self) -> Vec<String> {
        let res = self.req("GET", "/api/trash", &self.admin).send().unwrap();
        expect(res, 200)["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| s(&n["id"]).to_owned())
            .collect()
    }

    fn history_status(&self, id: &str) -> u16 {
        self.req("GET", &format!("/api/notes/{id}/history"), &self.admin)
            .send()
            .unwrap()
            .status()
            .as_u16()
    }

    fn asset_trash(&self) -> Vec<String> {
        let res = self
            .req("GET", "/api/assets/trash", &self.admin)
            .send()
            .unwrap();
        expect(res, 200)["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| s(&a["id"]).to_owned())
            .collect()
    }
}

fn expect(res: Response, status: u16) -> Value {
    let got = res.status().as_u16();
    let text = res.text().unwrap();
    assert_eq!(got, status, "unexpected status, body: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("{v} is not a string"))
}

#[test]
fn a_session_empties_the_trash_with_the_notes_history() {
    let api = Api::start();
    let only = api.upload("only.png");
    let shared = api.upload("shared.png");
    let a = api.trashed("First", Some(&only));
    let b = api.trashed("Second", Some(&shared));
    api.create(json!({ "title": "Still here", "file": shared }));
    assert_eq!(api.history_status(&a), 200);

    let res = api
        .req("POST", "/api/trash/empty", &api.session)
        .send()
        .unwrap();
    let body = expect(res, 200);
    let mut purged: Vec<&str> = body["purged"].as_array().unwrap().iter().map(s).collect();
    purged.sort();
    let mut want = [a.as_str(), b.as_str()];
    want.sort();
    assert_eq!(purged, want, "{body}");
    assert_eq!(body["assets_trashed"], json!([only]), "{body}");

    assert!(api.trash_ids().is_empty());
    assert_eq!(api.history_status(&a), 404);
    assert_eq!(api.history_status(&b), 404);

    // The asset only a purged note named went to the asset trash; the shared one stays.
    assert_eq!(api.asset_trash(), std::slice::from_ref(&only));
    let vault = api.server.vault_path();
    assert!(!vault.join("assets").join(&only).exists());
    assert!(vault.join("assets").join(&shared).exists());

    // A purged note cannot be restored.
    let res = api
        .req("POST", &format!("/api/trash/{a}/restore"), &api.session)
        .send()
        .unwrap();
    expect(res, 404);

    // Emptying an empty trash purges nothing.
    let res = api
        .req("POST", "/api/trash/empty", &api.session)
        .send()
        .unwrap();
    assert_eq!(expect(res, 200)["purged"], json!([]));
}

#[test]
fn a_session_purges_one_note_and_the_others_stay() {
    let api = Api::start();
    let a = api.trashed("First", None);
    let b = api.trashed("Second", None);

    let res = api
        .req("DELETE", &format!("/api/trash/{a}"), &api.session)
        .send()
        .unwrap();
    assert_eq!(expect(res, 200)["purged"], json!([a]));
    assert_eq!(api.trash_ids(), std::slice::from_ref(&b));
    assert_eq!(api.history_status(&a), 404);
    assert_eq!(api.history_status(&b), 200);

    // Purging it again, or a note never in the trash, is a 404.
    let res = api
        .req("DELETE", &format!("/api/trash/{a}"), &api.session)
        .send()
        .unwrap();
    expect(res, 404);
    let res = api
        .req("POST", &format!("/api/trash/{a}/restore"), &api.session)
        .send()
        .unwrap();
    expect(res, 404);

    // The other note still comes back.
    let res = api
        .req("POST", &format!("/api/trash/{b}/restore"), &api.session)
        .send()
        .unwrap();
    expect(res, 200);
}

#[test]
fn every_token_scope_is_refused_human_only_and_nothing_is_purged() {
    let api = Api::start();
    let a = api.trashed("First", None);
    let tokens = [
        ("read", api.token("reader", "read")),
        ("write", api.token("writer", "write")),
        ("admin", api.admin.clone()),
    ];
    for (scope, auth) in &tokens {
        for (method, path) in [
            ("POST", "/api/trash/empty".to_owned()),
            ("DELETE", format!("/api/trash/{a}")),
        ] {
            let res = api.req(method, &path, auth).send().unwrap();
            let body = expect(res, 403);
            assert_eq!(
                body["error"], "human_only",
                "{scope} {method} {path}: {body}"
            );
            assert_eq!(
                body["message"], "Only the signed-in user can empty the trash.",
                "{body}"
            );
        }
    }
    assert_eq!(api.trash_ids(), std::slice::from_ref(&a));
    assert_eq!(api.history_status(&a), 200);

    // A write token can still restore.
    let res = api
        .req("POST", &format!("/api/trash/{a}/restore"), &tokens[1].1)
        .send()
        .unwrap();
    expect(res, 200);
}
