//! `testkit-seed <vault>`: give a vault folder the test password and admin token, as
//! [`testkit::TestServer`] does, and print both as one JSON line. The browser tests (S5) run it
//! before they start `vault-server` on the folder.

use std::path::PathBuf;

fn main() {
    let Some(vault) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: testkit-seed <vault>");
        std::process::exit(2);
    };
    let admin_token = testkit::seed_credentials(&vault);
    println!(
        "{}",
        serde_json::json!({ "password": testkit::PASSWORD, "admin_token": admin_token })
    );
}
