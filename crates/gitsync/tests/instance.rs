//! One instance of the git sync tests: a `TestServer` configured and driven through
//! `/api/integrations/gitsync`, as Settings > Sync drives it.

use std::path::Path;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde_json::{Value, json};
use testkit::TestServer;

/// The part size the test servers split assets into, so a small asset takes several parts.
pub const PART_BYTES: usize = 4096;

/// A type both test vaults have.
pub const MEMO: &str = r#"label = "Memo"

field a {
  kind = "string"
}

field b {
  kind = "string"
}

field link {
  kind   = "reference"
  target = "memo"
}

field pic {
  kind = "asset"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "a"
    slot "b"
    slot "link"
    slot "pic"
    slot "body"
  }
}
"#;

/// The memo type with fields only the first instance knows, and another label.
pub const MEMO_PLUS: &str = r#"label = "Memo plus"

field a {
  kind = "string"
}

field b {
  kind = "string"
}

field link {
  kind   = "reference"
  target = "memo"
}

field pic {
  kind = "asset"
}

field body {
  kind = "content"
}

template default {
  tab "Main" {
    slot "title"
    slot "a"
    slot "b"
    slot "link"
    slot "pic"
    slot "body"
  }
}
"#;

/// A type only the first instance has, with a taxonomy only it has.
pub const RECIPE: &str = r#"label = "Recipe"

field course {
  kind     = "taxonomy"
  taxonomy = "courses"
}

template default {
  tab "Main" {
    slot "title"
    slot "course"
  }
}
"#;

pub const COURSES: &str = r#"label = "Courses"

term "Main" {}
term "Dessert" {}
"#;

/// The memo type alone.
pub fn seed(vault: &Path) {
    std::fs::create_dir_all(vault.join("types")).unwrap();
    std::fs::write(vault.join("types/memo.wcl"), MEMO).unwrap();
}

pub struct Instance {
    pub server: TestServer,
    pub name: String,
    http: Client,
}

impl Instance {
    /// An instance named `name` on a vault `seed` writes, with a small asset part size.
    pub fn start_seeded(name: &str, seed: impl FnOnce(&Path)) -> Instance {
        Instance::start_with(name, seed, &[])
    }

    /// An instance named `name` with the memo type.
    pub fn start(name: &str) -> Instance {
        Instance::start_seeded(name, seed)
    }

    /// As [`Instance::start_seeded`], with `env` for the server as well.
    pub fn start_with(name: &str, seed: impl FnOnce(&Path), env: &[(&str, &str)]) -> Instance {
        let part = PART_BYTES.to_string();
        let mut all = vec![("VAULT_GITSYNC_PART_BYTES", part.as_str())];
        all.extend_from_slice(env);
        Instance {
            server: TestServer::start_with_env(seed, &all),
            name: name.to_owned(),
            http: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
        }
    }

    /// An instance named `name` on a server already started.
    pub fn on(name: &str, server: TestServer) -> Instance {
        Instance {
            server,
            name: name.to_owned(),
            http: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
        }
    }

    /// Send `method path` with `body` and check the status; answers the JSON body, or `null`.
    pub fn call(&self, method: &str, path: &str, body: Option<Value>, status: u16) -> Value {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut rb = self
            .http
            .request(method.clone(), self.server.url(path))
            .bearer_auth(self.server.admin_token());
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        let res = rb.send().unwrap();
        let got = res.status().as_u16();
        let text = res.text().unwrap();
        assert_eq!(got, status, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    pub fn get(&self, path: &str) -> Value {
        self.call("GET", path, None, 200)
    }

    pub fn status(&self) -> Value {
        self.get("/api/integrations/gitsync")
    }

    pub fn set_key(&self, key: &str) {
        self.call(
            "PUT",
            "/api/integrations/gitsync/key",
            Some(json!({ "key": key })),
            204,
        );
    }

    /// Make a sync key here and answer it.
    pub fn generate_key(&self) -> String {
        let made = self.call("POST", "/api/integrations/gitsync/key/generate", None, 200);
        made["key"].as_str().unwrap().to_owned()
    }

    /// Save settings: `url` and the two queries, this instance's name, the schedule paused, and
    /// whatever `extra` adds.
    pub fn configure(&self, url: &str, export: &str, import: &str, extra: Value) {
        let mut settings = json!({
            "remote_url": url,
            "export_query": export,
            "import_query": import,
            "name": self.name,
            "paused": true,
        });
        if let (Value::Object(s), Value::Object(e)) = (&mut settings, extra) {
            s.extend(e);
        }
        self.call(
            "PUT",
            "/api/integrations/gitsync/settings",
            Some(settings),
            200,
        );
    }

    /// Run a round and wait for it to end; answers the status after it.
    pub fn round(&self) -> Value {
        self.call("POST", "/api/integrations/gitsync/run", None, 202);
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let status = self.status();
            if status["running"] == false {
                return status;
            }
            assert!(Instant::now() < deadline, "the round did not end: {status}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Run a round that must succeed; answers its report.
    pub fn sync(&self) -> Value {
        let status = self.round();
        assert_eq!(
            status["last_error"],
            Value::Null,
            "{} failed: {status}",
            self.name
        );
        status["last_report"].clone()
    }

    /// Create a memo from `wcl`; answers its id.
    pub fn create(&self, wcl: &str) -> String {
        let made = self.call(
            "POST",
            "/api/notes?type=memo",
            Some(json!({ "wcl": wcl })),
            201,
        );
        made["id"].as_str().unwrap().to_owned()
    }

    /// Store `bytes` as an asset named `filename`; answers its id.
    pub fn upload(&self, filename: &str, bytes: Vec<u8>) -> String {
        let res = self
            .http
            .post(self.server.url(&format!("/api/assets?filename={filename}")))
            .bearer_auth(self.server.admin_token())
            .body(bytes)
            .send()
            .unwrap();
        assert!(res.status().is_success(), "upload: {}", res.status());
        let v: Value = res.json().unwrap();
        s(&v["id"]).to_owned()
    }

    /// Asset `id`'s bytes, or `None` when this vault does not have it.
    pub fn asset(&self, id: &str) -> Option<Vec<u8>> {
        let res = self
            .http
            .get(self.server.url(&format!("/api/assets/{id}")))
            .bearer_auth(self.server.admin_token())
            .send()
            .unwrap();
        res.status()
            .is_success()
            .then(|| res.bytes().unwrap().to_vec())
    }

    /// Note `id`, or `None` when this vault does not have it live.
    pub fn note(&self, id: &str) -> Option<Value> {
        let res = self
            .http
            .get(self.server.url(&format!("/api/notes/{id}")))
            .bearer_auth(self.server.admin_token())
            .header("accept", "application/json")
            .send()
            .unwrap();
        match res.status().as_u16() {
            200 => Some(res.json().unwrap()),
            404 => None,
            s => panic!("GET note {id}: {s}"),
        }
    }

    /// Top-level field `name` of note `id`, `null` when it does not hold one.
    pub fn field(&self, id: &str, name: &str) -> Value {
        let note = self
            .note(id)
            .unwrap_or_else(|| panic!("{} has no note {id}", self.name));
        note["note"]["fields"][name].clone()
    }

    /// Send `method` to `path` of note `id` (`""`, `/archive`) at its current version.
    pub fn at_version(&self, method: &str, id: &str, path: &str) {
        let version = self.note(id).unwrap()["version"].clone();
        self.call(
            method,
            &format!("/api/notes/{id}{path}?base_version={}", s(&version)),
            None,
            200,
        );
    }

    /// Set top-level string field `name` of note `id` to `value`, keeping the rest.
    pub fn set(&self, id: &str, name: &str, value: &str) {
        let note = self.note(id).unwrap();
        let mut doc = note["note"].clone();
        doc["fields"][name] = json!(value);
        for stamp in ["created", "updated"] {
            if let Some(f) = doc["fields"].as_object_mut() {
                f.remove(stamp);
            }
        }
        self.call(
            "PUT",
            &format!("/api/notes/{id}"),
            Some(json!({ "note": doc, "base_version": note["version"] })),
            200,
        );
    }

    /// The authors of note `id`'s versions, oldest first, as `kind:name`.
    pub fn authors(&self, id: &str) -> Vec<String> {
        self.get(&format!("/api/notes/{id}/history"))["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("{}:{}", s(&v["author"]["kind"]), s(&v["author"]["name"])))
            .collect()
    }

    /// Whether note `id` is archived here.
    pub fn archived(&self, id: &str) -> bool {
        self.field(id, "archived") == true
    }

    /// The titles of this vault's notifications, newest first.
    pub fn notifications(&self) -> Vec<Value> {
        self.get("/api/notifications")["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// This vault's `gitsync.round` activity entries, newest first.
    pub fn rounds(&self) -> Vec<Value> {
        self.get("/api/activity?category=sync")["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "gitsync.round")
            .cloned()
            .collect()
    }
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// A memo's WCL with a title and fields a and b.
pub fn memo(title: &str, a: &str, b: &str) -> String {
    format!("title = \"{title}\"\na = \"{a}\"\nb = \"{b}\"\n")
}

/// The ids in report list `name`.
pub fn ids(report: &Value, name: &str) -> Vec<String> {
    report[name]
        .as_array()
        .unwrap_or_else(|| panic!("no {name} in {report}"))
        .iter()
        .map(|v| s(v).to_owned())
        .collect()
}

/// Two instances, `A` and `B`, sharing one sync key and the bare repo at `url`.
pub fn pair(url: &str, a_export: &str, b_import: &str) -> (Instance, Instance) {
    let a = Instance::start("A");
    let b = Instance::start("B");
    let key = a.generate_key();
    b.set_key(&key);
    a.configure(url, a_export, "", json!({}));
    b.configure(url, "", b_import, json!({}));
    (a, b)
}
