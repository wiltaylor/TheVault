//! The tray icon (D12): the unread badge from the home server, and a menu with Quick capture,
//! Open, "Restart to update" once an update is downloaded, the settings and Quit.
//!
//! Tray specifications have no badge, so the badge is drawn: a red dot on the icon while
//! anything is unread, the count beside the icon where the tray shows titles, and the count in
//! the tooltip.

use std::sync::Mutex;

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Wry};

use crate::settings::Settings;

/// The menu items' ids.
pub const CAPTURE: &str = "capture";
pub const OPEN: &str = "open";
pub const AUTOSTART: &str = "autostart";
pub const UPDATE_CHECK: &str = "update_check";
pub const INSTALL_ON_QUIT: &str = "install_on_quit";
pub const RESTART_TO_UPDATE: &str = "restart_to_update";
pub const QUIT: &str = "quit";

/// What the tray shows, for `/_desktop/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Badge {
    /// Whether the app has a tray icon at all.
    pub tray: bool,
    pub unread: u64,
    /// The text beside the icon, empty when nothing is unread.
    pub title: String,
    pub tooltip: String,
}

impl Badge {
    pub fn of(unread: u64) -> Badge {
        Badge {
            tray: false,
            unread,
            title: match unread {
                0 => String::new(),
                1..=99 => unread.to_string(),
                _ => "99+".to_owned(),
            },
            tooltip: match unread {
                0 => "TheVault".to_owned(),
                1 => "TheVault: 1 unread notification".to_owned(),
                n => format!("TheVault: {n} unread notifications"),
            },
        }
    }
}

pub struct Tray {
    icon: Option<TrayIcon>,
    autostart: Option<CheckMenuItem<Wry>>,
    update_check: Option<CheckMenuItem<Wry>>,
    install_on_quit: Option<CheckMenuItem<Wry>>,
    restart: Option<MenuItem<Wry>>,
    /// The version waiting for a restart, for the tooltip.
    ready: Mutex<Option<String>>,
    badge: Mutex<Badge>,
}

impl Tray {
    /// Puts the icon in the tray. Without a tray the app runs on, and says why on stderr.
    pub fn build(app: &AppHandle, settings: &Settings) -> Tray {
        match Tray::try_build(app, settings) {
            Ok(tray) => tray,
            Err(e) => {
                eprintln!("thevault: no tray icon: {e}");
                Tray {
                    icon: None,
                    autostart: None,
                    update_check: None,
                    install_on_quit: None,
                    restart: None,
                    ready: Mutex::new(None),
                    badge: Mutex::new(Badge::of(0)),
                }
            }
        }
    }

    fn try_build(app: &AppHandle, settings: &Settings) -> tauri::Result<Tray> {
        let autostart = CheckMenuItem::with_id(
            app,
            AUTOSTART,
            "Start on login",
            true,
            settings.autostart,
            None::<&str>,
        )?;
        let update_check = CheckMenuItem::with_id(
            app,
            UPDATE_CHECK,
            "Check for updates",
            true,
            settings.update_check,
            None::<&str>,
        )?;
        let install_on_quit = CheckMenuItem::with_id(
            app,
            INSTALL_ON_QUIT,
            "Install updates on quit",
            true,
            settings.install_on_quit,
            None::<&str>,
        )?;
        let restart = MenuItem::with_id(
            app,
            RESTART_TO_UPDATE,
            restart_label(None),
            false,
            None::<&str>,
        )?;
        let menu = Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, CAPTURE, "Quick capture", true, None::<&str>)?,
                &MenuItem::with_id(app, OPEN, "Open TheVault", true, None::<&str>)?,
                &restart,
                &PredefinedMenuItem::separator(app)?,
                &autostart,
                &update_check,
                &install_on_quit,
                &PredefinedMenuItem::separator(app)?,
                &MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?,
            ],
        )?;
        let icon = TrayIconBuilder::with_id("main")
            .icon(plain_icon())
            .tooltip("TheVault")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(|app, event| crate::on_tray_menu(app, event.id.as_ref()))
            .build(app)?;
        let mut badge = Badge::of(0);
        badge.tray = true;
        Ok(Tray {
            icon: Some(icon),
            autostart: Some(autostart),
            update_check: Some(update_check),
            install_on_quit: Some(install_on_quit),
            restart: Some(restart),
            ready: Mutex::new(None),
            badge: Mutex::new(badge),
        })
    }

    /// Shows that `version` is downloaded and waiting for a restart, or that nothing is.
    pub fn set_update_ready(&self, version: Option<&str>) {
        *self.ready.lock().unwrap_or_else(|p| p.into_inner()) = version.map(str::to_owned);
        if let Some(item) = &self.restart {
            let _ = item.set_text(restart_label(version));
            let _ = item.set_enabled(version.is_some());
        }
        let unread = self.badge().unread;
        self.set_unread(unread);
    }

    /// Shows `unread` on the icon.
    pub fn set_unread(&self, unread: u64) {
        let mut badge = Badge::of(unread);
        if let Some(version) = self
            .ready
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_deref()
        {
            badge.tooltip = format!("{}\n{}", badge.tooltip, ready_text(version));
        }
        if let Some(icon) = &self.icon {
            let applied = icon
                .set_icon(Some(if unread > 0 {
                    badged_icon()
                } else {
                    plain_icon()
                }))
                .and_then(|()| icon.set_title(Some(badge.title.as_str()).filter(|t| !t.is_empty())))
                .and_then(|()| icon.set_tooltip(Some(badge.tooltip.as_str())));
            match applied {
                Ok(()) => badge.tray = true,
                Err(e) => eprintln!("thevault: updating the tray: {e}"),
            }
        }
        *self.badge.lock().unwrap_or_else(|p| p.into_inner()) = badge;
    }

    pub fn badge(&self) -> Badge {
        self.badge.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Ticks the settings' menu items to match `settings`.
    pub fn show_settings(&self, settings: &Settings) {
        if let Some(item) = &self.autostart {
            let _ = item.set_checked(settings.autostart);
        }
        if let Some(item) = &self.update_check {
            let _ = item.set_checked(settings.update_check);
        }
        if let Some(item) = &self.install_on_quit {
            let _ = item.set_checked(settings.install_on_quit);
        }
    }
}

/// What the window and the tray say about an update waiting for a restart.
pub fn ready_text(version: &str) -> String {
    format!("TheVault {version} is ready \u{2014} restart to update")
}

/// The tray's restart item, which is greyed out until an update is ready.
fn restart_label(version: Option<&str>) -> String {
    match version {
        Some(version) => format!("Restart to update to {version}"),
        None => "No update waiting".to_owned(),
    }
}

fn plain_icon() -> Image<'static> {
    tauri::include_image!("icons/32x32.png")
}

/// The icon with a red dot in its lower right corner.
fn badged_icon() -> Image<'static> {
    let plain = plain_icon();
    let (w, h) = (plain.width(), plain.height());
    let mut rgba = plain.rgba().to_vec();
    dot(&mut rgba, w, h);
    Image::new_owned(rgba, w, h)
}

/// Paints a red dot with a white ring over the lower right of a `w` by `h` RGBA image.
fn dot(rgba: &mut [u8], w: u32, h: u32) {
    let r = w.min(h) as f32 * 0.3;
    let (cx, cy) = (w as f32 - r, h as f32 - r);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let colour = if d <= r - 1.5 {
                [0xd3, 0x2f, 0x2f, 0xff]
            } else if d <= r {
                [0xff, 0xff, 0xff, 0xff]
            } else {
                continue;
            };
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_counts_unread_notifications() {
        assert_eq!(Badge::of(0).title, "");
        assert_eq!(Badge::of(0).tooltip, "TheVault");
        assert_eq!(Badge::of(1).title, "1");
        assert_eq!(Badge::of(1).tooltip, "TheVault: 1 unread notification");
        assert_eq!(Badge::of(42).tooltip, "TheVault: 42 unread notifications");
        assert_eq!(Badge::of(100).title, "99+");
    }

    #[test]
    fn the_restart_item_names_the_update_waiting() {
        assert_eq!(restart_label(None), "No update waiting");
        assert_eq!(restart_label(Some("0.2.0")), "Restart to update to 0.2.0");
        assert_eq!(
            ready_text("0.2.0"),
            "TheVault 0.2.0 is ready \u{2014} restart to update"
        );
    }

    #[test]
    fn the_badged_icon_has_a_red_dot_in_its_corner() {
        let icon = badged_icon();
        let (w, h) = (icon.width(), icon.height());
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            icon.rgba()[i..i + 4].to_vec()
        };
        let r = (w.min(h) as f32 * 0.3) as u32;
        assert_eq!(px(w - r, h - r), [0xd3, 0x2f, 0x2f, 0xff]);
        assert_eq!(px(0, 0), plain_icon().rgba()[0..4].to_vec());
    }
}
