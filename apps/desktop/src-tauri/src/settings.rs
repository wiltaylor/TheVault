//! The app's own settings, in `settings.json` beside the local vault, and the autostart entry
//! they control (D12).
//!
//! "Start on login" writes an XDG autostart entry, `$XDG_CONFIG_HOME/autostart/dev.wiltaylor.thevault.desktop`,
//! that starts the app minimised to the tray; turning it off removes the entry. "Check for
//! updates" turns the update check on and off ([`crate::update`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The flag that starts the app with its window hidden, as the autostart entry does.
pub const MINIMISED_FLAG: &str = "--minimised";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Start minimised to the tray when the user logs in.
    pub autostart: bool,
    /// Check GitHub Releases for a newer app.
    pub update_check: bool,
    /// The newer version the user chose not to be told about again.
    pub skipped_update: Option<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            autostart: false,
            update_check: true,
            skipped_update: None,
        }
    }
}

fn settings_file(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// The settings saved in `dir`, or the defaults when there are none or they do not parse.
pub fn load(dir: &Path) -> Settings {
    std::fs::read(settings_file(dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Saves `settings` in `dir`, replacing the file in one rename.
pub fn save(dir: &Path, settings: &Settings) -> Result<(), String> {
    let path = settings_file(dir);
    let tmp = dir.join("settings.json.tmp");
    let text = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Where the autostart entry goes for the config folder `config`, normally `~/.config`.
pub fn autostart_file(config: &Path) -> PathBuf {
    config
        .join("autostart")
        .join(format!("{}.desktop", crate::links::APP_ID))
}

/// The autostart entry that starts `exe` minimised.
pub fn autostart_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TheVault\n\
         Comment=Start TheVault in the tray\n\
         Exec={} {MINIMISED_FLAG}\n\
         Icon=thevault\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        launcher_exec(exe)
    )
}

/// `exe` as a desktop entry's `Exec` key names it.
pub fn launcher_exec(exe: &Path) -> String {
    desktop_exec_arg(&exe.to_string_lossy())
}

/// Writes or removes the autostart entry under `config` to match `on`.
pub fn apply_autostart(config: &Path, exe: &Path, on: bool) -> Result<(), String> {
    let path = autostart_file(config);
    if on {
        let dir = path.parent().expect("the entry has a folder");
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, autostart_entry(exe)))
            .map_err(|e| format!("writing {}: {e}", path.display()))
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("removing {}: {e}", path.display())),
        }
    }
}

/// The program the autostart entry and the launcher start: the AppImage when the app runs from
/// one, since the binary it runs lives in a mount that goes away, and this binary otherwise.
pub fn launcher_exe() -> PathBuf {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("thevault"))
}

/// `arg` quoted for a desktop entry's `Exec` key when it needs it.
fn desktop_exec_arg(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        return arg.to_owned();
    }
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_default_when_missing_or_broken() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings::default());
        let changed = Settings {
            autostart: true,
            update_check: false,
            skipped_update: Some("0.2.0".into()),
        };
        save(dir.path(), &changed).unwrap();
        assert_eq!(load(dir.path()), changed);
        std::fs::write(dir.path().join("settings.json"), "not json").unwrap();
        assert_eq!(load(dir.path()), Settings::default());
    }

    #[test]
    fn autostart_writes_and_removes_its_entry() {
        let config = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/The Vault/thevault");
        apply_autostart(config.path(), exe, true).unwrap();
        let entry = std::fs::read_to_string(autostart_file(config.path())).unwrap();
        assert!(
            entry.contains("Exec=\"/opt/The Vault/thevault\" --minimised\n"),
            "{entry}"
        );
        apply_autostart(config.path(), exe, false).unwrap();
        assert!(!autostart_file(config.path()).exists());
        // Turning it off twice is fine.
        apply_autostart(config.path(), exe, false).unwrap();
    }

    #[test]
    fn exec_arguments_are_quoted_only_when_they_need_it() {
        assert_eq!(desktop_exec_arg("/usr/bin/thevault"), "/usr/bin/thevault");
        assert_eq!(desktop_exec_arg("/a b/$x\"y"), "\"/a b/\\$x\\\"y\"");
    }
}
