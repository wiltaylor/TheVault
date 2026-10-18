//! S1: the app builds the server hands out, from its clients folder, without credentials.

use std::ffi::OsStr;
use std::path::Path;

use clients::Publish;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    IF_NONE_MATCH, IF_RANGE, RANGE,
};
use serde_json::Value;
use testkit::TestServer;

const APK: &[u8] = b"PK\x03\x04 pretend android package bytes";
const APPIMAGE: &[u8] = b"\x7fELF pretend appimage";

/// Publish a release APK and AppImage into `dir`.
fn publish(dir: &Path) -> clients::Manifest {
    let src = tempfile::tempdir().unwrap();
    let apk = src.path().join("TheVault_0.3.0_aarch64.apk");
    let image = src.path().join("TheVault_0.3.0_amd64.AppImage");
    std::fs::write(&apk, APK).unwrap();
    std::fs::write(&image, APPIMAGE).unwrap();
    clients::publish(
        dir,
        &[Publish::file(apk), Publish::file(image)],
        1_700_000_000_000,
    )
    .unwrap()
}

fn serve_with(dir: &Path) -> TestServer {
    TestServer::start_with_args(|_| {}, &[OsStr::new("--clients"), dir.as_os_str()])
}

/// A request with no credentials at all.
fn get(server: &TestServer, path: &str) -> reqwest::blocking::Response {
    Client::new().get(server.url(path)).send().unwrap()
}

#[test]
fn the_manifest_is_public_and_empty_when_nothing_is_published() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve_with(&dir.path().join("not-there-yet"));
    let res = get(&server, "/api/clients");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.json::<Value>().unwrap(),
        serde_json::json!({ "clients": [], "enabled": true })
    );
    assert_eq!(
        get(&server, "/api/clients/TheVault_0.3.0_aarch64.apk").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&server, "/api/clients/latest/apk").status(),
        StatusCode::NOT_FOUND
    );
    // Everything else still needs credentials.
    assert_eq!(
        get(&server, "/api/notes").status(),
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn lists_and_downloads_published_builds_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve_with(dir.path());
    let manifest = publish(dir.path());

    let listed: Value = get(&server, "/api/clients").json().unwrap();
    let list = listed["clients"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    let apk = &list[0];
    assert_eq!(apk["platform"], "android");
    assert_eq!(apk["kind"], "apk");
    assert_eq!(apk["arch"], "aarch64");
    assert_eq!(apk["version"], "0.3.0");
    assert_eq!(apk["build"], "release");
    assert_eq!(apk["size"], APK.len());
    assert_eq!(apk["sha256"], manifest.clients[0].sha256);
    assert_eq!(apk["filename"], "TheVault_0.3.0_aarch64.apk");
    assert_eq!(apk["path"], "/api/clients/TheVault_0.3.0_aarch64.apk");
    assert_eq!(
        apk["url"],
        server.url("/api/clients/TheVault_0.3.0_aarch64.apk")
    );
    assert!(apk["qr_svg"].as_str().unwrap().contains("<svg"));
    let image = &list[1];
    assert_eq!(image["platform"], "linux");
    assert_eq!(image["kind"], "appimage");
    assert_eq!(image["arch"], "x86_64");
    assert!(image.get("qr_svg").is_none(), "only the APK has a QR code");

    let res = get(&server, "/api/clients/TheVault_0.3.0_aarch64.apk");
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers().clone();
    assert_eq!(h[CONTENT_TYPE], "application/vnd.android.package-archive");
    assert_eq!(h[CONTENT_LENGTH], APK.len().to_string().as_str());
    assert_eq!(h[ACCEPT_RANGES], "bytes");
    let etag = format!("\"{}\"", manifest.clients[0].sha256);
    assert_eq!(h[ETAG], etag.as_str());
    let disposition = h[CONTENT_DISPOSITION].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment;")
            && disposition.contains("TheVault_0.3.0_aarch64.apk"),
        "{disposition}"
    );
    assert_eq!(res.bytes().unwrap().as_ref(), APK);

    let res = get(&server, "/api/clients/TheVault_0.3.0_x86_64.AppImage");
    assert_eq!(res.headers()[CONTENT_TYPE], "application/octet-stream");
    assert_eq!(res.bytes().unwrap().as_ref(), APPIMAGE);

    // HEAD answers the length without the body.
    let head = Client::new()
        .head(server.url("/api/clients/TheVault_0.3.0_aarch64.apk"))
        .send()
        .unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        head.headers()[CONTENT_LENGTH],
        APK.len().to_string().as_str()
    );

    // One range, as a resumed download asks for it.
    let res = Client::new()
        .get(server.url("/api/clients/TheVault_0.3.0_aarch64.apk"))
        .header(RANGE, "bytes=4-9")
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()[CONTENT_RANGE],
        format!("bytes 4-9/{}", APK.len()).as_str()
    );
    assert_eq!(res.headers()[CONTENT_LENGTH], "6");
    assert_eq!(res.bytes().unwrap().as_ref(), &APK[4..10]);
    // The rest of a short download, as the Android updater resumes one: from its byte count to
    // the end, while the ETag (the SHA-256) still names the same file.
    let res = Client::new()
        .get(server.url("/api/clients/TheVault_0.3.0_aarch64.apk"))
        .header(RANGE, "bytes=10-")
        .header(IF_RANGE, &etag)
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()[CONTENT_RANGE],
        format!("bytes 10-{}/{}", APK.len() - 1, APK.len()).as_str()
    );
    assert_eq!(res.bytes().unwrap().as_ref(), &APK[10..]);
    // A file replaced since is sent whole.
    let res = Client::new()
        .get(server.url("/api/clients/TheVault_0.3.0_aarch64.apk"))
        .header(RANGE, "bytes=10-")
        .header(IF_RANGE, "\"an-older-build\"")
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.bytes().unwrap().as_ref(), APK);
    let res = Client::new()
        .get(server.url("/api/clients/TheVault_0.3.0_aarch64.apk"))
        .header(RANGE, "bytes=9999-")
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    // The SHA-256 ETag lets a client skip a build it already has.
    let res = Client::new()
        .get(server.url("/api/clients/TheVault_0.3.0_aarch64.apk"))
        .header(IF_NONE_MATCH, &etag)
        .send()
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);

    // The newest build of a kind, at an address that does not change.
    let res = get(&server, "/api/clients/latest/apk");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[ETAG], etag.as_str());
    assert_eq!(res.bytes().unwrap().as_ref(), APK);
    assert_eq!(
        get(&server, "/api/clients/latest/appimage?arch=x86_64").status(),
        StatusCode::OK
    );
    assert_eq!(
        get(&server, "/api/clients/latest/appimage?arch=aarch64").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&server, "/api/clients/latest/deb").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&server, "/api/clients/latest/exe").status(),
        StatusCode::NOT_FOUND
    );
}

#[test]
fn serves_only_the_files_the_manifest_lists() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve_with(&dir.path().join("clients"));
    publish(&dir.path().join("clients"));
    std::fs::write(dir.path().join("clients/unlisted.apk"), b"x").unwrap();
    std::fs::write(dir.path().join("secret.apk"), b"x").unwrap();
    for path in [
        "/api/clients/unlisted.apk",
        "/api/clients/manifest.json",
        "/api/clients/%2E%2E%2Fsecret.apk",
        "/api/clients/..%2Fsecret.apk",
        "/api/clients/.manifest.json.partial",
        "/api/clients/TheVault_0.3.0_aarch64.apk%00",
    ] {
        assert_eq!(get(&server, path).status(), StatusCode::NOT_FOUND, "{path}");
    }
    // Writing under the path is not a public route.
    let put = Client::new()
        .put(server.url("/api/clients/x.apk"))
        .body("x")
        .send()
        .unwrap();
    assert_eq!(put.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn the_folder_defaults_to_the_vaults_server_folder() {
    let server = TestServer::start();
    publish(&server.vault_path().join(".vault/clients"));
    let listed: Value = get(&server, "/api/clients").json().unwrap();
    assert_eq!(listed["clients"].as_array().unwrap().len(), 2);
    assert_eq!(
        get(&server, "/api/clients/TheVault_0.3.0_aarch64.apk")
            .bytes()
            .unwrap()
            .as_ref(),
        APK
    );
}

/// Publish a signed release AppImage of `version` into `dir`.
fn publish_signed(dir: &Path, version: &str, signature: &str) {
    let src = tempfile::tempdir().unwrap();
    let image = src
        .path()
        .join(format!("TheVault_{version}_amd64.AppImage"));
    std::fs::write(&image, APPIMAGE).unwrap();
    std::fs::write(
        src.path()
            .join(format!("TheVault_{version}_amd64.AppImage.sig")),
        signature,
    )
    .unwrap();
    clients::publish(dir, &[Publish::file(image)], 1_700_000_000_000).unwrap();
}

#[test]
fn the_updater_endpoint_offers_a_newer_signed_appimage_and_204_otherwise() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve_with(dir.path());
    let check = |path: &str| get(&server, &format!("/api/clients/update/{path}"));
    // Nothing published.
    assert_eq!(check("linux/x86_64/0.1.0").status(), StatusCode::NO_CONTENT);
    // An AppImage published without a signature is never offered: the updater would refuse it.
    publish(dir.path());
    assert_eq!(check("linux/x86_64/0.1.0").status(), StatusCode::NO_CONTENT);

    publish_signed(dir.path(), "0.4.0", "c2lnbmF0dXJlIG9mIDAuNC4w\n");
    let res = check("linux/x86_64/0.1.0");
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = res.json().unwrap();
    assert_eq!(body["version"], "0.4.0");
    assert_eq!(body["signature"], "c2lnbmF0dXJlIG9mIDAuNC4w");
    assert_eq!(body["pub_date"], "2023-11-14T22:13:20Z");
    assert_eq!(
        body["url"],
        server.url("/api/clients/TheVault_0.4.0_x86_64.AppImage")
    );
    assert!(body["notes"].as_str().unwrap().contains("0.4.0"));
    // The url downloads the AppImage itself.
    let bytes = get(&server, "/api/clients/TheVault_0.4.0_x86_64.AppImage")
        .bytes()
        .unwrap();
    assert_eq!(bytes.as_ref(), APPIMAGE);

    // Up to date, newer than the server's, another architecture or platform: nothing.
    for path in [
        "linux/x86_64/0.4.0",
        "linux/x86_64/1.0.0",
        "linux/aarch64/0.1.0",
        "windows/x86_64/0.1.0",
        "linux/x86_64/not-a-version",
    ] {
        assert_eq!(check(path).status(), StatusCode::NO_CONTENT, "{path}");
    }
    // A pre-release of the published version is older than it.
    assert_eq!(check("linux/x86_64/0.4.0-rc.1").status(), StatusCode::OK);
}
