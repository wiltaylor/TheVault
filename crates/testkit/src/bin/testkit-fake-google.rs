//! `testkit-fake-google`: run a [`testkit::FakeGoogle`] with its defaults on a free port and print
//! one JSON line, `{ "base_url", "host", "endpoints", "auth" }`, where `endpoints` is the
//! `endpoints` object of the `google_settings` secret and `auth` the fake consent page. The
//! browser tests (S5) start it and point the `google_settings` secret at it. It runs until its standard input closes or it is killed.

use std::io::Read;

fn main() {
    let google = testkit::FakeGoogle::start();
    println!(
        "{}",
        serde_json::json!({
            "base_url": google.base_url(),
            "host": google.host(),
            "endpoints": google.endpoints(),
            "auth": google.url(testkit::fake_google::AUTH_PATH),
        })
    );
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
}
