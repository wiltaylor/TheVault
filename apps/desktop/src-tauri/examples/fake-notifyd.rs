//! A stand-in notification daemon for the S6 tests: it owns `org.freedesktop.Notifications` on
//! the session bus and reports every call as a JSON line on stdout.
//!
//! - `{"ready": true}` once it owns the name;
//! - `{"notify": <id>, "app": ..., "summary": ..., "body": ..., "actions": [...]}` per
//!   notification raised;
//! - `{"close": <id>}` per notification an app closed.
//!
//! It reads commands from stdin, one per line, to act as the user: `dismiss <id>` closes a
//! notification as the user would, and `click <id>` clicks it.

use std::collections::HashMap;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

const PATH: &str = "/org/freedesktop/Notifications";

struct Daemon {
    next: u32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Daemon {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &mut self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id == 0 {
            self.next += 1;
            self.next
        } else {
            replaces_id
        };
        println!(
            "{}",
            json!({ "notify": id, "app": app_name, "summary": summary, "body": body, "actions": actions })
        );
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        println!("{}", json!({ "close": id }));
        Daemon::notification_closed(&emitter, id, 3).await?;
        Ok(())
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "fake-notifyd".into(),
            "TheVault".into(),
            "1".into(),
            "1.2".into(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let connection = zbus::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at(PATH, Daemon { next: 0 })?
        .build()
        .await?;
    println!("{}", json!({ "ready": true }));
    let iface = connection
        .object_server()
        .interface::<_, Daemon>(PATH)
        .await?;
    let emitter = iface.signal_emitter();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut words = line.split_whitespace();
        let (Some(command), Some(Ok(id))) = (words.next(), words.next().map(str::parse::<u32>))
        else {
            continue;
        };
        match command {
            "dismiss" => Daemon::notification_closed(emitter, id, 2).await?,
            "click" => {
                Daemon::action_invoked(emitter, id, "default").await?;
                Daemon::notification_closed(emitter, id, 2).await?;
            }
            _ => {}
        }
    }
    // Keep the name until the test stops the daemon.
    std::future::pending::<()>().await;
    Ok(())
}
