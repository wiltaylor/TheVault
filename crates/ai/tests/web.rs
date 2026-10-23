//! The chat's guarded fetcher against the fake web server (ai D27): the address rule on every
//! hop, the redirect limit, the size limits, the probe and the timeout.

use std::net::IpAddr;
use std::time::Duration;

use ai::web::{FetchError, Fetcher, LOCAL_NETWORK, MAX_REDIRECTS};
use testkit::{FAKE_PAGE, FakeWeb};

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// A guarded fetcher that treats `127.0.0.2` as a public address, so the fake web served there
/// can redirect to a local one.
fn guarded() -> Fetcher {
    Fetcher::exempting(vec![ip("127.0.0.2")], Duration::from_secs(10))
}

#[tokio::test]
async fn the_rule_refuses_local_addresses_and_fetch_private_lifts_it() {
    let web = FakeWeb::start();
    let port = web.addr().port();
    let f = Fetcher::new(false);
    for url in [
        format!("http://127.0.0.1:{port}/page.html"),
        "http://10.0.0.1/".to_owned(),
        "http://169.254.1.1/".to_owned(),
        format!("http://[::1]:{port}/page.html"),
        format!("http://[::ffff:127.0.0.1]:{port}/page.html"),
        // A name, refused by the resolver once it resolves to loopback.
        format!("http://localhost:{port}/page.html"),
    ] {
        let e = f.get(&url, 1024).await.unwrap_err();
        assert_eq!(e, FetchError::LocalNetwork, "{url}");
        assert_eq!(e.to_string(), LOCAL_NETWORK);
    }
    assert!(
        web.requests().is_empty(),
        "a refused fetch reached the server"
    );

    let open = Fetcher::new(true);
    assert!(open.private());
    let page = open.get(&web.url("/page.html"), 1 << 20).await.unwrap();
    assert_eq!(page.media_type.as_deref(), Some("text/html"));
    assert_eq!(page.body, FAKE_PAGE.as_bytes());
    let ua = web.requests()[0].header("user-agent").unwrap().to_owned();
    assert!(ua.starts_with("TheVault/"), "{ua}");
    assert!(
        ua.ends_with("(+https://github.com/wiltaylor/TheVault)"),
        "{ua}"
    );
}

#[tokio::test]
async fn a_redirect_to_a_local_address_is_refused() {
    let public = FakeWeb::start_on(ip("127.0.0.2"));
    let local = FakeWeb::start();
    let f = guarded();
    // The exempt address itself is fetched.
    let ok = f.get(&public.url("/page.html"), 1 << 20).await;
    assert!(ok.is_ok(), "{ok:?}");
    for target in [
        local.url("/page.html"),
        format!("http://localhost:{}/page.html", local.addr().port()),
    ] {
        let url = public.url(&format!("/to?url={target}"));
        let e = f.get(&url, 1 << 20).await.unwrap_err();
        assert_eq!(e, FetchError::LocalNetwork, "{target}");
        // The probe checks each hop the same way.
        assert_eq!(f.probe(&url).await.unwrap_err(), FetchError::LocalNetwork);
    }
    assert!(
        local.requests().is_empty(),
        "the redirect reached the local server"
    );
}

#[tokio::test]
async fn five_redirects_are_followed_and_a_sixth_is_refused() {
    let web = FakeWeb::start();
    let f = Fetcher::new(true);
    // /redirect/4 is five hops: 4, 3, 2, 1, 0, then the page.
    let got = f.get(&web.url("/redirect/4"), 1 << 20).await.unwrap();
    assert_eq!(got.url.as_str(), web.url("/page.html"));
    let e = f
        .get(&web.url(&format!("/redirect/{MAX_REDIRECTS}")), 1 << 20)
        .await
        .unwrap_err();
    assert_eq!(e, FetchError::TooManyRedirects);
}

#[tokio::test]
async fn a_long_body_is_cut_at_the_limit() {
    let web = FakeWeb::start();
    let f = Fetcher::new(true);
    let cut = f.get(&web.url("/bytes?size=5000"), 1000).await.unwrap();
    assert_eq!(cut.body.len(), 1000);
    assert!(cut.cut);
    assert_eq!(cut.size, Some(5000));
    let whole = f.get(&web.url("/bytes?size=1000"), 1000).await.unwrap();
    assert!(!whole.cut);
}

#[tokio::test]
async fn a_download_is_written_to_its_file_whatever_its_length() {
    let web = FakeWeb::start();
    let f = Fetcher::new(true);
    let dir = tempfile::tempdir().unwrap();
    for query in ["size=300000&type=application/zip", "size=300000&nolength=1"] {
        let path = dir.path().join(format!("{}.part", query.len()));
        let url = web.url(&format!("/bytes?{query}"));
        let to = web.url(&format!("/to?url={}", url.replace('&', "%26")));
        let got = f.download_to(&to, &path).await.unwrap();
        assert_eq!(got.size, 300_000);
        assert_eq!(got.url.as_str(), url);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 300_000);
    }
    // The file must be new.
    let path = dir
        .path()
        .join(format!("{}.part", "size=300000&nolength=1".len()));
    assert!(
        f.download_to(&web.url("/bytes?size=1"), &path)
            .await
            .is_err()
    );
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 300_000);
}

#[tokio::test]
async fn a_download_that_fails_leaves_no_file() {
    let web = FakeWeb::start();
    let f = Fetcher::new(true);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.part");
    // The connection closes after 1000 of the 5000 bytes it promised.
    let e = f
        .download_to(&web.url("/bytes?size=5000&drop=1000"), &path)
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::Failed(_)), "{e:?}");
    assert!(!path.exists());
    let e = f
        .download_to(&web.url("/missing"), &path)
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::Status(404, _)), "{e:?}");
    assert!(!path.exists());
    // Guarded, the address rule holds for a download too.
    let e = Fetcher::new(false)
        .download_to(&web.url("/bytes?size=10"), &path)
        .await
        .unwrap_err();
    assert_eq!(e, FetchError::LocalNetwork);
    assert!(!path.exists());
}

#[tokio::test]
async fn the_probe_uses_head_and_falls_back_to_a_ranged_get() {
    let web = FakeWeb::start();
    let f = Fetcher::new(true);
    let head = f
        .probe(&web.url("/bytes?size=1234&type=application/zip"))
        .await
        .unwrap();
    assert_eq!(head.size, Some(1234));
    assert_eq!(head.media_type.as_deref(), Some("application/zip"));
    assert!(head.body.is_empty());
    let ranged = f
        .probe(&web.url("/bytes?size=4321&nohead=1"))
        .await
        .unwrap();
    assert_eq!(ranged.size, Some(4321));
    let methods: Vec<String> = web
        .requests()
        .iter()
        .map(|r| format!("{} {}", r.method, r.header("range").unwrap_or("-")))
        .collect();
    assert_eq!(methods, ["HEAD -", "HEAD -", "GET bytes=0-0"]);
}

#[tokio::test]
async fn a_slow_answer_times_out() {
    let web = FakeWeb::start();
    let f = Fetcher::exempting(Vec::new(), Duration::from_millis(300));
    assert!(!f.private());
    // Guarded, the loopback server is refused before any wait.
    assert_eq!(
        f.get(&web.url("/slow?ms=2000"), 1 << 20).await.unwrap_err(),
        FetchError::LocalNetwork
    );
    let slow = FakeWeb::start_on(ip("127.0.0.2"));
    let f = Fetcher::exempting(vec![ip("127.0.0.2")], Duration::from_millis(300));
    assert_eq!(
        f.get(&slow.url("/slow?ms=2000"), 1 << 20)
            .await
            .unwrap_err(),
        FetchError::Timeout
    );
}

#[tokio::test]
async fn an_error_status_is_an_error() {
    let web = FakeWeb::start();
    let e = Fetcher::new(true)
        .get(&web.url("/missing"), 1024)
        .await
        .unwrap_err();
    assert!(matches!(e, FetchError::Status(404, _)), "{e:?}");
}
