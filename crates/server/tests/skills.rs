//! S1: Agent Skills through `/api/skills` (skills S1, S2, S3, S7): reads need `read`, every write
//! needs `admin`; `SKILL.md` is validated before it is written; a zip imports from one folder or
//! from its root, refuses an existing name unless `replace=true`, and writes nothing when it
//! fails; export round-trips; and each write is a `skill.change` in the activity log.

use std::io::{Read, Write};

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use testkit::TestServer;

const PDF: &str = "---\nname: pdf\ndescription: Fills PDF forms. Use when a PDF form arrives.\nlicense: MIT\ncompatibility: Requires python3\nmetadata:\n  author: me\nallowed-tools: Bash(python3:*) Read\nunknown: kept\n---\n\n# PDF\n\nRun scripts/fill.py.\n";

struct Api {
    server: TestServer,
    client: Client,
}

fn expect(res: Response, status: StatusCode) -> Value {
    let got = res.status();
    let text = res.text().unwrap();
    assert_eq!(got, status, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, bytes) in files {
        z.start_file(*path, opts).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// A `multipart/form-data` body holding `zip` as field `file`, as the web app sends it.
fn form(zip: &[u8]) -> (String, Vec<u8>) {
    let boundary = "skillboundary7MA4YWxkTrZu0gW";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skill.zip\"\r\nContent-Type: application/zip\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(zip);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

impl Api {
    fn start() -> Api {
        Api {
            server: TestServer::start(),
            client: Client::new(),
        }
    }

    fn req(&self, method: &str, path: &str, token: &str) -> RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.server.url(path))
            .header(AUTHORIZATION, format!("Bearer {token}"))
    }

    fn admin(&self, method: &str, path: &str) -> RequestBuilder {
        self.req(method, path, self.server.admin_token())
    }

    fn put_md(&self, name: &str, text: &str) -> Response {
        self.admin("PUT", &format!("/api/skills/{name}"))
            .header(CONTENT_TYPE, "text/markdown; charset=utf-8")
            .body(text.to_owned())
            .send()
            .unwrap()
    }

    fn import(&self, zip: &[u8], query: &str) -> Response {
        let (ty, body) = form(zip);
        self.admin("POST", &format!("/api/skills/import{query}"))
            .header(CONTENT_TYPE, ty)
            .body(body)
            .send()
            .unwrap()
    }

    fn token(&self, scope: &str) -> String {
        let res = self
            .admin("POST", "/api/tokens")
            .json(&json!({ "name": format!("t-{scope}"), "scope": scope }))
            .send()
            .unwrap();
        expect(res, StatusCode::CREATED)["secret"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn skill_entries(&self) -> Vec<String> {
        let page = expect(
            self.admin("GET", "/api/activity?kind=skill.change&limit=100")
                .send()
                .unwrap(),
            StatusCode::OK,
        );
        let mut out: Vec<String> = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                assert_eq!(e["category"], "definitions");
                e["summary"].as_str().unwrap().to_owned()
            })
            .collect();
        out.reverse();
        out
    }
}

#[test]
fn a_skill_is_created_read_listed_and_its_files_written_and_deleted() {
    let api = Api::start();
    let created = expect(api.put_md("pdf", PDF), StatusCode::CREATED);
    assert_eq!(created["name"], "pdf");
    assert_eq!(created["valid"], true);
    assert_eq!(created["license"], "MIT");
    assert_eq!(created["compatibility"], "Requires python3");
    assert_eq!(created["metadata"], json!({ "author": "me" }));
    assert_eq!(created["allowed_tools"], "Bash(python3:*) Read");
    assert_eq!(created["extra"], json!({ "unknown": "kept" }));
    assert_eq!(
        created["issues"],
        json!([{
            "at": "unknown",
            "message": "\"unknown\" is not a field of the Agent Skills specification; it is kept and ignored",
            "severity": "warning",
        }])
    );
    assert_eq!(created["body"], "\n# PDF\n\nRun scripts/fill.py.\n");
    assert_eq!(
        std::fs::read_to_string(api.server.vault_path().join("skills/pdf/SKILL.md")).unwrap(),
        PDF
    );
    expect(api.put_md("pdf", PDF), StatusCode::OK);

    let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0, 0, 255, 254];
    let put = expect(
        api.admin("PUT", "/api/skills/pdf/files/assets/logo.png")
            .body(png.clone())
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(
        put["file"],
        json!({ "path": "assets/logo.png", "size": 8, "media_type": "image/png", "text": false })
    );
    expect(
        api.admin("PUT", "/api/skills/pdf/files/scripts/fill.py")
            .body("print('filled')\n")
            .send()
            .unwrap(),
        StatusCode::OK,
    );

    let list = expect(
        api.admin("GET", "/api/skills").send().unwrap(),
        StatusCode::OK,
    );
    let s = &list["skills"][0];
    assert_eq!(s["name"], "pdf");
    assert_eq!(s["files"], 3);
    assert_eq!(s["file_count"], 3);
    assert_eq!(s["size"], PDF.len() + 8 + 16);
    assert_eq!(s["valid"], true);
    assert_eq!(
        s["description"],
        "Fills PDF forms. Use when a PDF form arrives."
    );
    assert!(s.get("body").is_none());

    let one = expect(
        api.admin("GET", "/api/skills/pdf").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(
        one["files"],
        json!([
            { "path": "SKILL.md", "size": PDF.len(), "media_type": "text/markdown", "text": true },
            { "path": "assets/logo.png", "size": 8, "media_type": "image/png", "text": false },
            { "path": "scripts/fill.py", "size": 16, "media_type": "text/x-python", "text": true },
        ])
    );

    let res = api
        .admin("GET", "/api/skills/pdf/files/assets/logo.png")
        .send()
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()[CONTENT_TYPE], "image/png");
    assert!(
        res.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("sandbox")
    );
    assert_eq!(res.bytes().unwrap().to_vec(), png);
    let res = api
        .admin("GET", "/api/skills/pdf/files/SKILL.md")
        .send()
        .unwrap();
    assert_eq!(res.headers()[CONTENT_TYPE], "text/markdown; charset=utf-8");
    assert_eq!(res.text().unwrap(), PDF);
    expect(
        api.admin("GET", "/api/skills/pdf/files/nope.txt")
            .send()
            .unwrap(),
        StatusCode::NOT_FOUND,
    );
    expect(
        api.admin("GET", "/api/skills/nope").send().unwrap(),
        StatusCode::NOT_FOUND,
    );

    // SKILL.md cannot go alone; a folder goes with its files.
    expect(
        api.admin("DELETE", "/api/skills/pdf/files/SKILL.md")
            .send()
            .unwrap(),
        StatusCode::BAD_REQUEST,
    );
    let gone = expect(
        api.admin("DELETE", "/api/skills/pdf/files/scripts")
            .send()
            .unwrap(),
        StatusCode::OK,
    );
    assert_eq!(
        gone,
        json!({ "name": "pdf", "deleted": ["scripts/fill.py"] })
    );
    assert!(!api.server.vault_path().join("skills/pdf/scripts").exists());
    expect(
        api.admin("DELETE", "/api/skills/pdf").send().unwrap(),
        StatusCode::OK,
    );
    assert!(!api.server.vault_path().join("skills/pdf").exists());
    let list = expect(
        api.admin("GET", "/api/skills").send().unwrap(),
        StatusCode::OK,
    );
    assert_eq!(list["skills"], json!([]));

    assert_eq!(
        api.skill_entries(),
        [
            "Created skill pdf",
            "Changed skill pdf",
            "Changed skill pdf: wrote assets/logo.png",
            "Changed skill pdf: wrote scripts/fill.py",
            "Changed skill pdf: deleted scripts",
            "Deleted skill pdf",
        ]
    );
}

#[test]
fn an_invalid_skill_md_is_refused_with_its_issues_and_nothing_is_written() {
    let api = Api::start();
    let err = expect(
        api.put_md(
            "pdf",
            "---\nname: other\ndescription: \"\"\nmetadata:\n  v: 1\n---\n",
        ),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(err["error"], "invalid");
    let at: Vec<&str> = err["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["at"].as_str().unwrap())
        .collect();
    assert_eq!(at, ["metadata.v", "name", "description"]);
    let err = expect(api.put_md("pdf", "no frontmatter"), StatusCode::BAD_REQUEST);
    assert_eq!(err["issues"][0]["at"], "frontmatter");
    expect(
        api.put_md("Bad_Name", "---\nname: Bad_Name\ndescription: d\n---\n"),
        StatusCode::BAD_REQUEST,
    );
    // A file needs its skill.
    expect(
        api.admin("PUT", "/api/skills/pdf/files/a.txt")
            .body("x")
            .send()
            .unwrap(),
        StatusCode::NOT_FOUND,
    );
    assert!(!api.server.vault_path().join("skills/pdf").exists());
    assert!(api.skill_entries().is_empty());
}

#[test]
fn read_tokens_read_and_only_admin_writes() {
    let api = Api::start();
    expect(api.put_md("pdf", PDF), StatusCode::CREATED);
    let read = api.token("read");
    let write = api.token("write");
    for token in [&read, &write] {
        expect(
            api.req("GET", "/api/skills", token).send().unwrap(),
            StatusCode::OK,
        );
        expect(
            api.req("GET", "/api/skills/pdf", token).send().unwrap(),
            StatusCode::OK,
        );
        let res = api
            .req("GET", "/api/skills/pdf/files/SKILL.md", token)
            .send()
            .unwrap();
        assert_eq!(res.status(), 200);
        let res = api
            .req("GET", "/api/skills/pdf/export", token)
            .send()
            .unwrap();
        assert_eq!(res.status(), 200);
        for (method, path) in [
            ("PUT", "/api/skills/pdf"),
            ("DELETE", "/api/skills/pdf"),
            ("PUT", "/api/skills/pdf/files/a.txt"),
            ("DELETE", "/api/skills/pdf/files/SKILL.md"),
            ("POST", "/api/skills/import"),
        ] {
            let res = api.req(method, path, token).body(PDF).send().unwrap();
            assert_eq!(res.status(), 403, "{method} {path}");
        }
    }
    let doc = expect(
        api.admin("GET", "/api/openapi.json").send().unwrap(),
        StatusCode::OK,
    );
    let paths = &doc["paths"];
    assert_eq!(paths["/api/skills"]["get"]["x-scope"], "read");
    assert_eq!(paths["/api/skills/import"]["post"]["x-scope"], "admin");
    assert_eq!(paths["/api/skills/{name}"]["put"]["x-scope"], "admin");
    assert_eq!(
        paths["/api/skills/{name}/files/{*path}"]["get"]["x-scope"],
        "read"
    );
    assert_eq!(
        paths["/api/skills/{name}/files/{*path}"]["get"]["parameters"][1]["name"],
        "path"
    );
}

#[test]
fn a_zip_imports_from_a_folder_or_its_root_and_replaces_only_when_asked() {
    let api = Api::start();
    let binary: Vec<u8> = (0..=255u8).rev().collect();
    // A folder named otherwise: the frontmatter names the skill.
    let folder = zip(&[
        ("pdf-1.0/SKILL.md", PDF.as_bytes()),
        ("pdf-1.0/scripts/fill.py", b"print(1)\n"),
        ("pdf-1.0/assets/blob.bin", &binary),
    ]);
    let created = expect(api.import(&folder, ""), StatusCode::CREATED);
    assert_eq!(created["name"], "pdf");
    assert_eq!(created["replaced"], false);
    assert_eq!(created["files"].as_array().unwrap().len(), 3);
    assert_eq!(
        std::fs::read(api.server.vault_path().join("skills/pdf/assets/blob.bin")).unwrap(),
        binary
    );

    // SKILL.md at the root, as a raw body: the name exists, so it is refused.
    let root = zip(&[
        ("SKILL.md", PDF.as_bytes()),
        ("references/REFERENCE.md", b"# Ref\n"),
    ]);
    let err = expect(
        api.admin("POST", "/api/skills/import")
            .header(CONTENT_TYPE, "application/zip")
            .body(root.clone())
            .send()
            .unwrap(),
        StatusCode::CONFLICT,
    );
    assert_eq!(err["error"], "skill_exists");
    assert_eq!(err["name"], "pdf");
    assert!(
        api.server
            .vault_path()
            .join("skills/pdf/scripts/fill.py")
            .is_file()
    );
    let replaced = expect(api.import(&root, "?replace=true"), StatusCode::OK);
    assert_eq!(replaced["replaced"], true);
    let paths: Vec<&str> = replaced["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["SKILL.md", "references/REFERENCE.md"]);
    assert!(!api.server.vault_path().join("skills/pdf/scripts").exists());

    assert_eq!(
        api.skill_entries(),
        [
            "Imported skill pdf from a zip",
            "Replaced skill pdf from a zip"
        ]
    );
}

#[test]
fn an_invalid_zip_is_refused_before_anything_is_written() {
    let api = Api::start();
    for (n, bytes) in [
        b"not a zip at all".to_vec(),
        zip(&[("pdf/readme.md", b"no skill")]),
        zip(&[
            ("a/SKILL.md", PDF.as_bytes()),
            ("b/SKILL.md", PDF.as_bytes()),
        ]),
        zip(&[
            ("pdf/SKILL.md", b"---\nname: pdf\n---\n# no description\n"),
            ("pdf/scripts/run.sh", b"echo\n"),
        ]),
        zip(&[("SKILL.md", PDF.as_bytes()), ("../../escape.txt", b"x")]),
    ]
    .iter()
    .enumerate()
    {
        let res = api.import(bytes, "?replace=true");
        assert_eq!(res.status(), 400, "case {n}: {}", res.text().unwrap());
    }
    let invalid = expect(
        api.import(&zip(&[("pdf/SKILL.md", b"---\nname: pdf\n---\n")]), ""),
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(invalid["error"], "invalid");
    assert_eq!(invalid["issues"][0]["at"], "description");
    assert_eq!(
        std::fs::read_dir(api.server.vault_path().join("skills"))
            .unwrap()
            .count(),
        0
    );
    assert!(!api.server.vault_path().join("escape.txt").exists());
    assert!(api.skill_entries().is_empty());
}

#[test]
fn an_export_imports_back_as_it_was() {
    let api = Api::start();
    let binary: Vec<u8> = (0..=255u8).collect();
    let folder = zip(&[
        ("pdf/SKILL.md", PDF.as_bytes()),
        ("pdf/assets/blob.bin", &binary),
        ("pdf/scripts/fill.py", b"print(1)\n"),
    ]);
    expect(api.import(&folder, ""), StatusCode::CREATED);
    let res = api.admin("GET", "/api/skills/pdf/export").send().unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()[CONTENT_TYPE], "application/zip");
    assert!(
        res.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("pdf.zip")
    );
    let exported = res.bytes().unwrap().to_vec();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&exported)).unwrap();
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        names.push(f.name().unwrap().into_owned());
        if f.name().unwrap() == "pdf/assets/blob.bin" {
            assert_eq!(bytes, binary);
        }
    }
    assert_eq!(
        names,
        ["pdf/SKILL.md", "pdf/assets/blob.bin", "pdf/scripts/fill.py"]
    );
    let before = expect(
        api.admin("GET", "/api/skills/pdf").send().unwrap(),
        StatusCode::OK,
    );
    expect(
        api.admin("DELETE", "/api/skills/pdf").send().unwrap(),
        StatusCode::OK,
    );
    let mut after = expect(api.import(&exported, ""), StatusCode::CREATED);
    after.as_object_mut().unwrap().remove("replaced");
    assert_eq!(after, before);
}
