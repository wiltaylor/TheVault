//! Desktop notifications through the session's notification daemon, the
//! `org.freedesktop.Notifications` D-Bus service (D9, D12).
//!
//! The notifier raises a server notification, closes it again by the server's id, and reports
//! what the user did with it: closed it, or clicked it.

use std::collections::HashMap;
use std::pin::Pin;

use futures_util::{Stream, StreamExt};
use zbus::zvariant::Value;

use crate::live::Item;

/// The name the notifications carry.
const APP_NAME: &str = "TheVault";

/// The icon name the notifications carry, which the package installs.
const ICON: &str = "thevault";

/// The action a click on the notification's body sends.
const DEFAULT_ACTION: &str = "default";

/// Why the daemon closed a notification, from the specification.
const CLOSED_EXPIRED: u32 = 1;
const CLOSED_DISMISSED: u32 = 2;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;
}

/// What the user or the daemon did with a notification, by the server's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// The user closed it.
    Dismissed(String),
    /// The user clicked it.
    Clicked(String),
    /// It timed out, or went away for another reason.
    Expired(String),
}

/// A daemon signal, by the daemon's id.
enum Raw {
    Closed(u32, u32),
    Action(u32, String),
}

/// The daemon signal `message` carries, when it is one the notifier reads.
fn raw(message: &zbus::Message) -> Option<Raw> {
    let header = message.header();
    match header.member()?.as_str() {
        "NotificationClosed" => {
            let (id, reason): (u32, u32) = message.body().deserialize().ok()?;
            Some(Raw::Closed(id, reason))
        }
        "ActionInvoked" => {
            let (id, key): (u32, String) = message.body().deserialize().ok()?;
            Some(Raw::Action(id, key))
        }
        _ => None,
    }
}

pub struct Notifier {
    proxy: NotificationsProxy<'static>,
    signals: Pin<Box<dyn Stream<Item = Raw> + Send>>,
    /// Server id to daemon id, for what the desktop shows now.
    shown: HashMap<String, u32>,
}

impl Notifier {
    /// Connects to the session bus. The daemon itself need not be running yet.
    pub async fn connect() -> zbus::Result<Notifier> {
        let connection = zbus::Connection::session().await?;
        let proxy = NotificationsProxy::new(&connection).await?;
        // One stream for both signals, so they arrive in the order the daemon sent them: a click
        // is an `ActionInvoked` followed by a `NotificationClosed`, and handling the close first
        // would lose the click.
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.Notifications")?
            .path("/org/freedesktop/Notifications")?
            .build();
        let signals = zbus::MessageStream::for_match_rule(rule, &connection, Some(64))
            .await?
            .filter_map(|message| async move { raw(&message.ok()?) });
        Ok(Notifier {
            proxy,
            signals: Box::pin(signals),
            shown: HashMap::new(),
        })
    }

    /// Raises `item` on the desktop.
    pub async fn show(&mut self, item: &Item) -> zbus::Result<()> {
        let body = item.body.as_deref().unwrap_or_default();
        let mut hints = HashMap::new();
        hints.insert("desktop-entry", Value::from(crate::links::APP_ID));
        let id = self
            .proxy
            .notify(
                APP_NAME,
                0,
                ICON,
                &item.title,
                body,
                &[DEFAULT_ACTION, "Open"],
                hints,
                -1,
            )
            .await?;
        self.shown.insert(item.id.clone(), id);
        Ok(())
    }

    /// Closes the notification raised for the server's `id`, when the desktop still shows it.
    pub async fn close(&mut self, id: &str) {
        if let Some(daemon_id) = self.shown.remove(id)
            && let Err(e) = self.proxy.close_notification(daemon_id).await
        {
            eprintln!("thevault: closing notification {id}: {e}");
        }
    }

    /// The next thing the user or the daemon did with a notification the app raised. Signals
    /// about other apps' notifications, and about ones the app closed itself, are skipped.
    pub async fn next_signal(&mut self) -> Signal {
        loop {
            let Some(raw) = self.signals.next().await else {
                return std::future::pending().await;
            };
            let daemon_id = match &raw {
                Raw::Closed(id, _) | Raw::Action(id, _) => *id,
            };
            let Some(server_id) = self
                .shown
                .iter()
                .find_map(|(s, d)| (*d == daemon_id).then(|| s.clone()))
            else {
                continue;
            };
            let signal = match raw {
                Raw::Action(_, key) if key == DEFAULT_ACTION => Signal::Clicked(server_id),
                Raw::Action(..) => continue,
                Raw::Closed(_, CLOSED_DISMISSED) => Signal::Dismissed(server_id),
                Raw::Closed(_, CLOSED_EXPIRED) | Raw::Closed(..) => Signal::Expired(server_id),
            };
            let (Signal::Dismissed(id) | Signal::Clicked(id) | Signal::Expired(id)) = &signal;
            self.shown.remove(id);
            return signal;
        }
    }
}
