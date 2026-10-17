//! The application menu entry an AppImage writes for itself (D12).
//!
//! Wayland compositors find a window's icon by its `app_id`, which is the app's identifier,
//! [`APP_ID`] ([`name_program`]): they look for `<APP_ID>.desktop`, or an
//! entry whose `StartupWMClass` is the `app_id`, and show its `Icon`. The deb and rpm packages
//! install their entry and icons from `linux/thevault.desktop`; an AppImage run directly installs
//! nothing, so the window would get the compositor's generic icon.
//!
//! While "Add TheVault to the application menu" is on (the default), an app running from an
//! AppImage writes `<data>/applications/<APP_ID>.desktop`, which starts the AppImage and handles
//! `thevault://` links, and the icons as `<data>/icons/hicolor/<size>x<size>/apps/<APP_ID>.png`.
//! `<data>` is `$XDG_DATA_HOME`, normally `~/.local/share`. Each start rewrites only what changed,
//! so a moved AppImage gets its `Exec` rewritten. Turning the setting off removes them.

use std::path::{Path, PathBuf};

use crate::links::{APP_ID, SCHEME};

/// The bundled icons the entry installs, by size in pixels.
const ICONS: &[(u32, &[u8])] = &[
    (32, include_bytes!("../icons/32x32.png")),
    (64, include_bytes!("../icons/64x64.png")),
    (128, include_bytes!("../icons/128x128.png")),
    (256, include_bytes!("../icons/128x128@2x.png")),
    (512, include_bytes!("../icons/icon.png")),
];

/// The 256 px icon the window sets, for X11 and the compositors that read it.
pub fn window_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/128x128@2x.png")
}

/// Names the program [`APP_ID`] before GTK starts. GTK 3 sets a Wayland window's `app_id` to the
/// program's name, `g_get_prgname()`, which is otherwise the binary's, `thevault`; the GTK
/// application id (`enableGTKAppId`) only reaches GNOME, through its own protocol. On X11 it is
/// the window's `WM_CLASS` name too.
pub fn name_program() {
    glib::set_prgname(Some(APP_ID));
}

/// The AppImage the app runs from, or `None` for any other install.
pub fn appimage() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// The entry's path under the data folder `data`.
pub fn entry_file(data: &Path) -> PathBuf {
    data.join("applications").join(format!("{APP_ID}.desktop"))
}

fn icon_file(data: &Path, size: u32) -> PathBuf {
    hicolor(data)
        .join(format!("{size}x{size}"))
        .join("apps")
        .join(format!("{APP_ID}.png"))
}

fn hicolor(data: &Path) -> PathBuf {
    data.join("icons").join("hicolor")
}

/// The entry that starts the AppImage `exe`.
pub fn entry(exe: &Path) -> String {
    let exe = crate::settings::launcher_exec(exe);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TheVault\n\
         Comment=Notes, offline, synced with your TheVault server\n\
         Exec={exe} %u\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Office;\n\
         StartupWMClass={APP_ID}\n\
         MimeType=x-scheme-handler/{SCHEME};\n\
         Actions=capture;\n\
         \n\
         [Desktop Action capture]\n\
         Name=Quick capture\n\
         Exec={exe} {}\n",
        crate::CAPTURE_FLAG
    )
}

/// Writes the entry for `exe` and the icons under `data`, each only when it differs from what is
/// there. Says whether anything changed.
pub fn install(data: &Path, exe: &Path) -> Result<bool, String> {
    let mut changed = write_if_changed(&entry_file(data), entry(exe).as_bytes())?;
    for (size, png) in ICONS {
        changed |= write_if_changed(&icon_file(data, *size), png)?;
    }
    Ok(changed)
}

/// Removes the entry and the icons under `data`. Says whether there was anything to remove.
pub fn remove(data: &Path) -> Result<bool, String> {
    let mut removed = remove_file(&entry_file(data))?;
    for (size, _) in ICONS {
        removed |= remove_file(&icon_file(data, *size))?;
    }
    Ok(removed)
}

/// Installs or removes the entry under `data` to match `on`, then, when anything changed, tells
/// the desktop. Does nothing unless the app runs from an AppImage.
pub fn apply(data: &Path, on: bool) {
    let Some(exe) = appimage() else {
        return;
    };
    let changed = if on {
        install(data, &exe)
    } else {
        remove(data)
    };
    match changed {
        Ok(true) => refresh(data, on),
        Ok(false) => {}
        Err(e) => eprintln!("thevault: the application menu entry: {e}"),
    }
}

/// Tells the desktop the entry and the icons changed. Each tool is best effort: a desktop
/// without one notices the change itself, or at the next log-in.
fn refresh(data: &Path, installed: bool) {
    let applications = data.join("applications");
    // The icon theme's caches look at the folder's modification time.
    if let Ok(dir) = std::fs::File::open(hicolor(data)) {
        let _ = dir.set_modified(std::time::SystemTime::now());
    }
    let mut commands: Vec<(&str, Vec<String>)> = vec![(
        "update-desktop-database",
        vec![applications.to_string_lossy().into_owned()],
    )];
    if installed {
        commands.push((
            "xdg-mime",
            vec![
                "default".to_owned(),
                format!("{APP_ID}.desktop"),
                format!("x-scheme-handler/{SCHEME}"),
            ],
        ));
    }
    // KDE Plasma's menu and task manager read the entries through its own cache.
    commands.push(("kbuildsycoca6", vec![]));
    for (cmd, args) in commands {
        let _ = std::process::Command::new(cmd)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(false);
    }
    let dir = path.parent().expect("the file has a folder");
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&tmp, bytes))
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(true)
}

fn remove_file(path: &Path) -> Result<bool, String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("removing {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str = "/home/u/Apps/TheVault_0.1.2_amd64.AppImage";

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_entry_starts_the_appimage_under_the_app_id() {
        let entry = entry(Path::new(IMAGE));
        for line in [
            "[Desktop Entry]\n",
            "Name=TheVault\n",
            &format!("Exec={IMAGE} %u\n"),
            "Icon=dev.wiltaylor.thevault\n",
            "StartupWMClass=dev.wiltaylor.thevault\n",
            "MimeType=x-scheme-handler/thevault;\n",
            "Categories=Office;\n",
            &format!("Exec={IMAGE} --capture\n"),
        ] {
            assert!(entry.contains(line), "{line:?} missing from\n{entry}");
        }
        // A path with a space is quoted.
        let spaced = super::entry(Path::new("/home/u/My Apps/TheVault.AppImage"));
        assert!(
            spaced.contains("Exec=\"/home/u/My Apps/TheVault.AppImage\" %u\n"),
            "{spaced}"
        );
    }

    #[test]
    fn install_writes_the_entry_and_every_icon_once() {
        let data = tempfile::tempdir().unwrap();
        let data = data.path();
        assert!(install(data, Path::new(IMAGE)).unwrap());
        assert_eq!(
            entry_file(data),
            data.join("applications/dev.wiltaylor.thevault.desktop")
        );
        assert_eq!(read(&entry_file(data)), entry(Path::new(IMAGE)));
        for size in [32, 64, 128, 256, 512] {
            let icon = data.join(format!(
                "icons/hicolor/{size}x{size}/apps/dev.wiltaylor.thevault.png"
            ));
            let png = std::fs::read(&icon).unwrap();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{}", icon.display());
            let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
            assert_eq!(width, size, "{}", icon.display());
        }
        // Nothing changed, so nothing is written.
        assert!(!install(data, Path::new(IMAGE)).unwrap());
        // A stray temporary file never stays behind.
        let names: Vec<_> = std::fs::read_dir(data.join("applications"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["dev.wiltaylor.thevault.desktop"]);
    }

    #[test]
    fn a_moved_appimage_and_an_old_entry_are_rewritten() {
        let data = tempfile::tempdir().unwrap();
        let data = data.path();
        // What 0.1.2 wrote: the generic icon name and class.
        std::fs::create_dir_all(data.join("applications")).unwrap();
        std::fs::write(
            entry_file(data),
            "[Desktop Entry]\nExec=/old/TheVault.AppImage %u\nIcon=thevault\nStartupWMClass=thevault\n",
        )
        .unwrap();
        assert!(install(data, Path::new(IMAGE)).unwrap());
        assert_eq!(read(&entry_file(data)), entry(Path::new(IMAGE)));
        let moved = Path::new("/opt/TheVault.AppImage");
        assert!(install(data, moved).unwrap());
        assert!(read(&entry_file(data)).contains("Exec=/opt/TheVault.AppImage %u\n"));
        assert!(!install(data, moved).unwrap());
    }

    #[test]
    fn remove_takes_the_entry_and_the_icons_away() {
        let data = tempfile::tempdir().unwrap();
        let data = data.path();
        // Another app's icon in the same folder stays.
        let other = data.join("icons/hicolor/64x64/apps/other.png");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, b"png").unwrap();
        install(data, Path::new(IMAGE)).unwrap();
        assert!(remove(data).unwrap());
        assert!(!entry_file(data).exists());
        for size in [32, 64, 128, 256, 512] {
            assert!(!icon_file(data, size).exists());
        }
        assert!(other.exists());
        // Removing twice is fine.
        assert!(!remove(data).unwrap());
    }
}
