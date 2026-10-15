//! The global capture hotkey through the XDG GlobalShortcuts portal (D12).
//!
//! The app opens a portal session and binds one shortcut, `capture`, suggesting
//! [`PREFERRED_TRIGGER`]. The desktop decides the key: KDE asks the user the first time, and
//! Hyprland binds it in its own config
//! (`bind = SUPER SHIFT, N, global, dev.wiltaylor.thevault:capture`). When the portal is missing
//! or refuses, the user binds a key to `thevault --capture` in their desktop's keyboard settings
//! instead (`docs/desktop.md`).

use std::sync::{Arc, Mutex};

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use serde::Serialize;

/// The shortcut's id.
const SHORTCUT: &str = "capture";

/// The key the app suggests, in the shortcuts specification's notation.
const PREFERRED_TRIGGER: &str = "CTRL+ALT+N";

/// Where the hotkey stands, for `/_desktop/status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "detail")]
pub enum Status {
    /// The portal session is being set up.
    Starting,
    /// The portal bound the shortcut; the detail is the key it says triggers it.
    Bound(String),
    /// No portal shortcut: why, and the user binds `thevault --capture` instead.
    Unavailable(String),
}

/// Binds the capture shortcut and calls `capture` each time it fires, for as long as the app
/// runs. `status` follows how it went.
pub fn start(status: Arc<Mutex<Status>>, capture: impl Fn() + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        let outcome = bind(&status, capture).await;
        if let Err(e) = outcome {
            *status.lock().unwrap_or_else(|p| p.into_inner()) =
                Status::Unavailable(format!("{e}; bind a key to `thevault --capture` instead"));
        }
    });
}

async fn bind(status: &Mutex<Status>, capture: impl Fn()) -> ashpd::Result<()> {
    // A host app names itself to the portal first, on portals new enough to ask; older ones
    // take the id from the desktop file the process was started from.
    if let Ok(id) = crate::links::APP_ID.parse()
        && let Err(e) = ashpd::register_host_app(id).await
    {
        eprintln!("thevault: registering with the portal: {e}");
    }
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let shortcut = NewShortcut::new(SHORTCUT, "Quick capture in TheVault")
        .preferred_trigger(PREFERRED_TRIGGER);
    let bound = portal
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await?
        .response()?;
    let trigger = bound
        .shortcuts()
        .iter()
        .find(|s| s.id() == SHORTCUT)
        .map(|s| s.trigger_description().to_owned())
        .unwrap_or_default();
    *status.lock().unwrap_or_else(|p| p.into_inner()) = Status::Bound(trigger);
    let mut activated = portal.receive_activated().await?;
    while let Some(event) = activated.next().await {
        if event.shortcut_id() == SHORTCUT {
            capture();
        }
    }
    // Keep the session alive as long as the stream; it closes with the app.
    drop(session);
    Ok(())
}
