//! An AppImage installs itself in `~/Applications` (D12).
//!
//! When the app starts from an AppImage anywhere else, such as the download folder, it copies
//! the AppImage to `~/Applications/TheVault.AppImage`, starts that copy with the same arguments
//! and exits, before Tauri starts. The copy then writes the application menu entry
//! ([`crate::app_menu`]), and the self-updater replaces the copy in place, since it updates the
//! running `$APPIMAGE`.
//!
//! - With no installed copy, or an older one, the running AppImage replaces it.
//! - With one as new or newer, the running AppImage just starts it.
//! - The original is deleted when it sits in the download folder (`XDG_DOWNLOAD_DIR`, normally
//!   `~/Downloads`) or the temporary folder; anywhere else it stays, and the notice says so.
//!
//! After an install the copy shows a notice in its window once, "TheVault was installed to
//! ~/Applications and added to your apps", with Undo, which removes the menu entry and keeps the
//! file. The notice waits in `<data>/thevault/installed.json` until the user answers it.
//! `THEVAULT_NO_SELF_INSTALL=1` turns all of this off, for the release smoke test.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use semver::Version;
use serde::{Deserialize, Serialize};

/// The installed AppImage's file name, in [`install_dir`].
pub const INSTALL_NAME: &str = "TheVault.AppImage";

/// The flag that prints the version and exits, which the install asks an installed copy.
pub const VERSION_FLAG: &str = "--version";

/// What a start from an AppImage does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Run this copy: it is the installed one.
    Stay,
    /// Copy this AppImage over the installed one, which is missing or older, then start it.
    Install,
    /// The installed copy is as new as this one or newer: start it.
    Launch,
}

/// The notice an install leaves for the installed copy's window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// Where the AppImage was installed, with `~` for the home folder.
    pub path: String,
    /// The AppImage the user started, with `~` for the home folder.
    pub original: String,
    /// Whether the original was deleted.
    pub removed_original: bool,
}

/// Where the app installs itself under the home folder `home`.
pub fn install_dir(home: &Path) -> PathBuf {
    home.join("Applications")
}

/// What to do for the AppImage `running`, version `ours`, given the installed copy `installed`,
/// whose version `installed_version` reads.
pub fn plan(
    running: &Path,
    installed: &Path,
    ours: &Version,
    installed_version: impl FnOnce() -> Option<Version>,
) -> Plan {
    if same_file(running, installed) {
        return Plan::Stay;
    }
    if !installed.is_file() {
        return Plan::Install;
    }
    match installed_version() {
        Some(theirs) if theirs >= *ours => Plan::Launch,
        // Older, or a file that cannot say: this AppImage replaces it.
        _ => Plan::Install,
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Whether the original AppImage `original` may be deleted once installed: only from the
/// download folder `downloads` or one of the temporary folders `tmp`. Under the home folder
/// `home`, only the download folder counts, even for a home inside a temporary folder.
pub fn removable(original: &Path, home: &Path, downloads: &Path, tmp: &[PathBuf]) -> bool {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    let original = real(original);
    let inside = |dir: &Path| {
        let dir = real(dir);
        original.starts_with(&dir) && original != dir
    };
    if inside(downloads) {
        return true;
    }
    !inside(home) && tmp.iter().any(|dir| inside(dir))
}

/// The download folder: `XDG_DOWNLOAD_DIR` in `<config>/user-dirs.dirs`, else `~/Downloads`.
pub fn download_dir(home: &Path, config: &Path) -> PathBuf {
    std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let value = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=")?;
                let value = value.trim().trim_matches('"');
                let path = match value.strip_prefix("$HOME") {
                    Some(rest) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(value),
                };
                path.is_absolute().then_some(path)
            })
        })
        .unwrap_or_else(|| home.join("Downloads"))
}

/// `path` with `~` for the home folder `home`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Copies `src` to `dest`, executable, replacing what is there in one rename.
pub fn copy_into_place(src: &Path, dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = dest.parent().expect("the AppImage has a folder");
    let tmp = dir.join(format!(".{INSTALL_NAME}.{}.tmp", std::process::id()));
    let copied = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::copy(src, &tmp))
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|()| std::fs::rename(&tmp, dest));
    copied.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("installing {} to {}: {e}", src.display(), dest.display())
    })
}

/// The environment changes that keep the running AppImage's mount out of a program it starts:
/// each variable naming a path under `appdir` loses that path, and goes when nothing is left.
/// The AppImage's own variables go too; the started AppImage sets its own.
pub fn clean_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    appdir: Option<&Path>,
) -> Vec<(OsString, Option<OsString>)> {
    let mut out = Vec::new();
    for (key, value) in vars {
        if ["APPIMAGE", "APPDIR", "ARGV0", "OWD"]
            .iter()
            .any(|k| key == *k)
        {
            out.push((key, None));
            continue;
        }
        let Some(appdir) = appdir.filter(|d| !d.as_os_str().is_empty()) else {
            continue;
        };
        let text = value.to_string_lossy();
        let appdir = appdir.to_string_lossy();
        if !text.contains(appdir.as_ref()) {
            continue;
        }
        let kept: Vec<&str> = text
            .split(':')
            .filter(|part| !part.is_empty() && !part.starts_with(appdir.as_ref()))
            .collect();
        out.push((key, (!kept.is_empty()).then(|| kept.join(":").into())));
    }
    out
}

fn command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    for (key, value) in clean_env(std::env::vars_os(), appdir.as_deref()) {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
    cmd
}

/// The version the AppImage `exe` reports for [`VERSION_FLAG`], or `None` when it says nothing
/// in time.
fn version_of(exe: &Path) -> Option<Version> {
    let mut child = command(exe)
        .arg(VERSION_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    parse_version(&out)
}

/// The version in [`VERSION_FLAG`]'s answer, `TheVault <version>`.
pub fn parse_version(out: &str) -> Option<Version> {
    let rest = out.lines().next()?.trim().strip_prefix("TheVault ")?;
    Version::parse(rest.trim()).ok()
}

/// The notice file in the app's folder `dir`.
fn notice_file(dir: &Path) -> PathBuf {
    dir.join("installed.json")
}

/// The install notice waiting in the app's folder `dir`.
pub fn notice(dir: &Path) -> Option<Notice> {
    let bytes = std::fs::read(notice_file(dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Leaves `notice` for the installed copy.
pub fn save_notice(dir: &Path, notice: &Notice) -> Result<(), String> {
    let path = notice_file(dir);
    std::fs::create_dir_all(dir)
        .and_then(|()| {
            std::fs::write(
                &path,
                serde_json::to_vec_pretty(notice).expect("a notice serialises"),
            )
        })
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// The user answered the notice.
pub fn clear_notice(dir: &Path) {
    let _ = std::fs::remove_file(notice_file(dir));
}

/// Where the app's own files live under the data folder.
fn app_dir(data: &Path) -> PathBuf {
    data.join("thevault")
}

/// Runs the install for a start from an AppImage. Answers `true` when another copy was started
/// and this process should exit.
pub fn settle(args: &[String]) -> bool {
    if std::env::var_os("THEVAULT_NO_SELF_INSTALL").is_some_and(|v| v == "1") {
        return false;
    }
    let Some(running) = crate::app_menu::appimage() else {
        return false;
    };
    let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
    else {
        return false;
    };
    let xdg = |var: &str, default: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(default))
    };
    let data = xdg("XDG_DATA_HOME", ".local/share");
    let config = xdg("XDG_CONFIG_HOME", ".config");
    let installed = install_dir(&home).join(INSTALL_NAME);
    let ours = Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate has a version");
    let plan = plan(&running, &installed, &ours, || version_of(&installed));
    if plan == Plan::Stay {
        return false;
    }
    if plan == Plan::Install
        && let Err(e) = copy_into_place(&running, &installed)
    {
        eprintln!("thevault: {e}; running from {}", running.display());
        return false;
    }
    let tmp = [std::env::temp_dir(), PathBuf::from("/tmp")];
    let removed_original = removable(&running, &home, &download_dir(&home, &config), &tmp)
        && std::fs::remove_file(&running).is_ok();
    if plan == Plan::Install {
        let notice = Notice {
            path: tilde(&install_dir(&home), &home),
            original: tilde(&running, &home),
            removed_original,
        };
        if let Err(e) = save_notice(&app_dir(&data), &notice) {
            eprintln!("thevault: {e}");
        }
    }
    match command(&installed).args(&args[1.min(args.len())..]).spawn() {
        Ok(_) => true,
        Err(e) => {
            eprintln!("thevault: starting {}: {e}", installed.display());
            // This copy is still mounted and can run, even if its file was deleted.
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// A temporary home with a downloaded AppImage in it.
    struct Home {
        dir: tempfile::TempDir,
    }

    impl Home {
        fn new() -> Home {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("Downloads")).unwrap();
            std::fs::write(
                dir.path().join("Downloads/TheVault_0.2.0_amd64.AppImage"),
                b"new",
            )
            .unwrap();
            Home { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn download(&self) -> PathBuf {
            self.path().join("Downloads/TheVault_0.2.0_amd64.AppImage")
        }

        fn installed(&self) -> PathBuf {
            install_dir(self.path()).join(INSTALL_NAME)
        }

        fn install(&self, bytes: &[u8]) {
            std::fs::create_dir_all(install_dir(self.path())).unwrap();
            std::fs::write(self.installed(), bytes).unwrap();
        }
    }

    #[test]
    fn a_first_start_installs_and_the_installed_copy_stays() {
        let home = Home::new();
        let asked = std::cell::Cell::new(false);
        let ask = || {
            asked.set(true);
            None
        };
        assert_eq!(
            plan(&home.download(), &home.installed(), &v("0.2.0"), ask),
            Plan::Install
        );
        // Nothing is installed, so nothing is asked its version.
        assert!(!asked.get());
        copy_into_place(&home.download(), &home.installed()).unwrap();
        assert_eq!(
            plan(
                &home.installed(),
                &home.installed(),
                &v("0.2.0"),
                || panic!("asked")
            ),
            Plan::Stay
        );
        // The same file through another path is still the installed copy.
        let via = home
            .path()
            .join("Applications/../Applications")
            .join(INSTALL_NAME);
        assert_eq!(
            plan(&via, &home.installed(), &v("0.2.0"), || panic!("asked")),
            Plan::Stay
        );
    }

    #[test]
    fn an_older_installed_copy_is_replaced_and_a_newer_one_started() {
        let home = Home::new();
        home.install(b"old");
        let ours = v("0.2.0");
        let run = |theirs: Option<&str>| {
            plan(&home.download(), &home.installed(), &ours, || theirs.map(v))
        };
        assert_eq!(run(Some("0.1.2")), Plan::Install);
        assert_eq!(run(Some("0.2.0-rc.1")), Plan::Install);
        assert_eq!(run(None), Plan::Install);
        assert_eq!(run(Some("0.2.0")), Plan::Launch);
        assert_eq!(run(Some("0.3.0")), Plan::Launch);
    }

    #[test]
    fn the_copy_is_executable_and_replaces_the_old_one_whole() {
        use std::os::unix::fs::PermissionsExt;
        let home = Home::new();
        home.install(b"old");
        copy_into_place(&home.download(), &home.installed()).unwrap();
        assert_eq!(std::fs::read(home.installed()).unwrap(), b"new");
        let mode = std::fs::metadata(home.installed())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let names: Vec<_> = std::fs::read_dir(install_dir(home.path()))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [INSTALL_NAME]);
    }

    #[test]
    fn only_a_download_or_a_temporary_file_is_deleted() {
        let home = tempfile::tempdir().unwrap();
        let downloads = download_dir(home.path(), &home.path().join(".config"));
        assert_eq!(downloads, home.path().join("Downloads"));
        let home = Path::new("/home/u");
        let downloads = Path::new("/home/u/Downloads");
        let tmp = [PathBuf::from("/var/tmp/u"), PathBuf::from("/tmp")];
        let removable = |p: &str| removable(Path::new(p), home, downloads, &tmp);
        assert!(removable("/home/u/Downloads/TheVault.AppImage"));
        assert!(removable("/home/u/Downloads/a/b.AppImage"));
        assert!(removable("/var/tmp/u/TheVault.AppImage"));
        assert!(removable("/tmp/TheVault.AppImage"));
        assert!(!removable("/home/u/bin/TheVault.AppImage"));
        assert!(!removable("/home/u/Downloads"));
        assert!(!removable("/home/u/DownloadsX/T.AppImage"));
        assert!(!removable("/opt/TheVault.AppImage"));
        // A home inside a temporary folder: only its download folder counts.
        let home = Path::new("/tmp/h");
        let downloads = Path::new("/tmp/h/Downloads");
        assert!(super::removable(
            Path::new("/tmp/h/Downloads/T.AppImage"),
            home,
            downloads,
            &tmp
        ));
        assert!(!super::removable(
            Path::new("/tmp/h/bin/T.AppImage"),
            home,
            downloads,
            &tmp
        ));
        assert!(super::removable(
            Path::new("/tmp/other/T.AppImage"),
            home,
            downloads,
            &tmp
        ));
    }

    #[test]
    fn the_download_folder_comes_from_user_dirs() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("user-dirs.dirs"),
            "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n",
        )
        .unwrap();
        assert_eq!(
            download_dir(home.path(), &config),
            home.path().join("Téléchargements")
        );
        std::fs::write(
            config.join("user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"/data/dl\"\n",
        )
        .unwrap();
        assert_eq!(
            download_dir(home.path(), &config),
            PathBuf::from("/data/dl")
        );
    }

    #[test]
    fn the_version_answer_parses() {
        assert_eq!(parse_version("TheVault 0.1.3\n"), Some(v("0.1.3")));
        assert_eq!(parse_version("TheVault 1.0.0-rc.2"), Some(v("1.0.0-rc.2")));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("<html>"), None);
    }

    #[test]
    fn the_started_copy_does_not_inherit_this_mount() {
        let appdir = Path::new("/tmp/.mount_TheVauABC");
        let vars = [
            ("APPIMAGE", "/home/u/Downloads/TheVault.AppImage"),
            ("APPDIR", "/tmp/.mount_TheVauABC"),
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_TheVauABC/usr/lib/:/tmp/.mount_TheVauABC/lib/:",
            ),
            (
                "XDG_DATA_DIRS",
                "/tmp/.mount_TheVauABC/usr/share:/usr/share:/usr/local/share",
            ),
            ("GTK_PATH", "/tmp/.mount_TheVauABC//usr/lib/gtk-3.0"),
            ("HOME", "/home/u"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let mut changes = clean_env(vars, Some(appdir));
        changes.sort();
        let want: Vec<(OsString, Option<OsString>)> = vec![
            ("APPDIR".into(), None),
            ("APPIMAGE".into(), None),
            ("GTK_PATH".into(), None),
            ("LD_LIBRARY_PATH".into(), None),
            (
                "XDG_DATA_DIRS".into(),
                Some("/usr/share:/usr/local/share".into()),
            ),
        ];
        assert_eq!(changes, want);
    }

    #[test]
    fn the_notice_waits_until_answered() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(notice(dir.path()), None);
        let home = Path::new("/home/u");
        let n = Notice {
            path: tilde(&install_dir(home), home),
            original: tilde(&home.join("bin/TheVault.AppImage"), home),
            removed_original: false,
        };
        assert_eq!(n.path, "~/Applications");
        assert_eq!(n.original, "~/bin/TheVault.AppImage");
        save_notice(dir.path(), &n).unwrap();
        assert_eq!(notice(dir.path()), Some(n));
        clear_notice(dir.path());
        assert_eq!(notice(dir.path()), None);
        assert_eq!(tilde(Path::new("/opt/x"), home), "/opt/x");
    }
}
