//! A large asset passes through the server without the server holding it (ticket 142, D19): an
//! upload streams into `.vault/incoming` and is committed from there, and a download streams
//! from the asset's file.

use std::io::Read;

use reqwest::blocking::{Body, Client};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use testkit::TestServer;

const SIZE: u64 = 300 * 1024 * 1024;
const LIMIT: u64 = 64 * 1024 * 1024;

/// A process's peak resident memory, in bytes, from `/proc/<pid>/status`.
fn peak_rss(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    let kb: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kb * 1024
}

/// The byte at `i` of the test file.
fn byte(i: u64) -> u8 {
    (i % 251) as u8
}

/// `len` bytes of the test file, made as they are read.
struct Pattern {
    at: u64,
    len: u64,
}

impl Read for Pattern {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (buf.len() as u64).min(self.len - self.at) as usize;
        for (k, b) in buf[..n].iter_mut().enumerate() {
            *b = byte(self.at + k as u64);
        }
        self.at += n as u64;
        Ok(n)
    }
}

fn client(token: &str) -> Client {
    let mut auth = HeaderMap::new();
    auth.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    Client::builder()
        .default_headers(auth)
        .timeout(None)
        .build()
        .unwrap()
}

fn put(server: &TestServer, c: &Client, id: &str, len: u64) -> reqwest::blocking::Response {
    c.put(server.url(&format!("/api/assets/{id}?filename=big.bin")))
        .body(Body::sized(Pattern { at: 0, len }, len))
        .send()
        .unwrap()
}

#[test]
fn a_300_mb_asset_goes_up_and_down_without_the_server_holding_it() {
    let server = TestServer::start();
    let c = client(server.admin_token());
    let pid = server.pid();

    // A small upload first, so the server's first-request setup is in the baseline.
    let res = put(&server, &c, "01J00000000000000000000001", 1024);
    assert_eq!(res.status(), 201, "{}", res.text().unwrap());
    let before = peak_rss(pid);

    let id = "01J00000000000000000000002";
    let res = put(&server, &c, id, SIZE);
    assert_eq!(res.status(), 201, "{}", res.text().unwrap());
    let grew = peak_rss(pid).saturating_sub(before);
    assert!(grew < LIMIT, "the upload grew the server by {grew} bytes");
    let stored = server.vault_path().join(format!("assets/{id}/big.bin"));
    assert_eq!(std::fs::metadata(&stored).unwrap().len(), SIZE);
    let incoming = server.vault_path().join(".vault/incoming");
    let left: Vec<_> = std::fs::read_dir(&incoming)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "left in incoming: {left:?}");

    // The same file again under its id is 200, and still streams.
    let res = put(&server, &c, id, SIZE);
    assert_eq!(res.status(), 200, "{}", res.text().unwrap());

    let mut res = c
        .get(server.url(&format!("/api/assets/{id}")))
        .send()
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.content_length(), Some(SIZE));
    let mut buf = vec![0u8; 1024 * 1024];
    let mut at = 0u64;
    loop {
        let n = res.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        for (k, b) in buf[..n].iter().enumerate() {
            assert_eq!(*b, byte(at + k as u64), "byte {}", at + k as u64);
        }
        at += n as u64;
    }
    assert_eq!(at, SIZE);
    let grew = peak_rss(pid).saturating_sub(before);
    assert!(grew < LIMIT, "the download grew the server by {grew} bytes");

    // A range from the middle streams too.
    let res = c
        .get(server.url(&format!("/api/assets/{id}")))
        .header("Range", "bytes=1000-1009")
        .send()
        .unwrap();
    assert_eq!(res.status(), 206);
    let want: Vec<u8> = (1000..1010).map(byte).collect();
    assert_eq!(res.bytes().unwrap().to_vec(), want);
}
