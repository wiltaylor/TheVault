//! The app builds a server hands out: the Android APK and the Linux AppImage, deb and rpm.
//!
//! They live in one folder, by default `<vault>/.vault/clients`, outside every folder that syncs
//! or is indexed. Beside them, `manifest.json` lists each build:
//!
//! ```json
//! { "clients": [ { "platform": "android", "kind": "apk", "arch": "aarch64",
//!     "version": "0.1.0", "build": "release", "size": 31457280,
//!     "sha256": "…", "filename": "TheVault_0.1.0_aarch64.apk", "published": 1760000000000 } ] }
//! ```
//!
//! [`publish`] copies builds in and rewrites the manifest; `vault clients publish` and
//! `just publish-clients` call it. The server reads the manifest with [`read`] and serves only the
//! files it lists, through [`find`]. The folder holds one build per platform, kind and
//! architecture: publishing a build replaces the one before it.

use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The manifest's file name inside the clients folder.
pub const MANIFEST: &str = "manifest.json";

/// One published build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// `android` or `linux`.
    pub platform: String,
    /// `apk`, `appimage`, `deb` or `rpm`.
    pub kind: String,
    /// `aarch64`, `x86_64`, `armv7`, `x86` or `universal`.
    pub arch: String,
    /// The app's version, such as `0.1.0`.
    pub version: String,
    /// `release` or `debug`.
    pub build: String,
    /// The file's size in bytes.
    pub size: u64,
    /// The file's SHA-256, as lowercase hex.
    pub sha256: String,
    /// The file's name inside the clients folder.
    pub filename: String,
    /// When it was published, in milliseconds since the epoch.
    pub published: i64,
}

/// `manifest.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub clients: Vec<Entry>,
}

/// The kinds of build a server hands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Apk,
    AppImage,
    Deb,
    Rpm,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Apk, Kind::AppImage, Kind::Deb, Kind::Rpm];

    /// The name the manifest records: `apk`, `appimage`, `deb` or `rpm`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Apk => "apk",
            Kind::AppImage => "appimage",
            Kind::Deb => "deb",
            Kind::Rpm => "rpm",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The platform it installs on.
    pub fn platform(self) -> &'static str {
        match self {
            Kind::Apk => "android",
            _ => "linux",
        }
    }

    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Apk => "apk",
            Kind::AppImage => "AppImage",
            Kind::Deb => "deb",
            Kind::Rpm => "rpm",
        }
    }

    /// The media type a download is served with.
    pub fn content_type(self) -> &'static str {
        match self {
            Kind::Apk => "application/vnd.android.package-archive",
            Kind::AppImage => "application/octet-stream",
            Kind::Deb => "application/vnd.debian.binary-package",
            Kind::Rpm => "application/x-rpm",
        }
    }

    /// The architecture a device of this kind most likely has, offered first.
    pub fn usual_arch(self) -> &'static str {
        match self {
            Kind::Apk => "aarch64",
            _ => "x86_64",
        }
    }

    /// Its place in the manifest: Android first, then the Linux builds.
    fn order(self) -> usize {
        Kind::ALL
            .iter()
            .position(|k| *k == self)
            .unwrap_or(usize::MAX)
    }
}

/// The kind of build a file name ends in, by its extension.
pub fn kind_of(filename: &str) -> Option<Kind> {
    let ext = filename.rsplit_once('.')?.1.to_ascii_lowercase();
    Kind::ALL
        .into_iter()
        .find(|k| k.extension().eq_ignore_ascii_case(&ext))
}

/// The architecture a build's file name names, such as `aarch64` in `app-arm64-debug.apk` or
/// `x86_64` in `TheVault_0.1.0_amd64.AppImage`.
pub fn arch_of(filename: &str) -> Option<&'static str> {
    let name = filename.to_ascii_lowercase();
    if name.contains("aarch64") || name.contains("arm64") {
        Some("aarch64")
    } else if name.contains("x86_64") || name.contains("amd64") {
        Some("x86_64")
    } else if name.contains("armv7") || name.contains("armeabi") {
        Some("armv7")
    } else if name.contains("i686") || name.contains("x86") {
        Some("x86")
    } else if name.contains("universal") {
        Some("universal")
    } else {
        None
    }
}

/// The version a build's file name names: the first `_`- or `-`-separated part that is three
/// dot-separated numbers, such as `0.1.0` in `TheVault_0.1.0_aarch64.apk`.
pub fn version_of(filename: &str) -> Option<String> {
    filename
        .split(['_', '-'])
        .find(|part| is_version(part))
        .map(str::to_owned)
}

fn is_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `name` is a plain file name the folder may hold: no path separators, no leading dot,
/// nothing outside printable ASCII, and not the manifest.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && name != MANIFEST
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// A failed read or publish.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn io_err(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error(format!("{}: {e}", path.display()))
}

/// The manifest in `dir`. A folder or manifest that does not exist is an empty manifest.
pub fn read(dir: &Path) -> Result<Manifest, Error> {
    let path = dir.join(MANIFEST);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| Error(format!("{}: not a clients manifest: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(io_err(&path)(e)),
    }
}

/// The entry `filename` names and the file's path, when the manifest lists it and the file is
/// there. A name the manifest does not list is never looked up on disk.
pub fn find(dir: &Path, filename: &str) -> Result<Option<(Entry, PathBuf)>, Error> {
    if !is_safe_name(filename) {
        return Ok(None);
    }
    let manifest = read(dir)?;
    let Some(entry) = manifest
        .clients
        .into_iter()
        .find(|e| e.filename == filename)
    else {
        return Ok(None);
    };
    let path = dir.join(&entry.filename);
    Ok(path.is_file().then_some((entry, path)))
}

/// The build of `kind` to offer: the one for `arch` when asked, else the kind's usual
/// architecture, else the first listed.
pub fn latest<'a>(manifest: &'a Manifest, kind: Kind, arch: Option<&str>) -> Option<&'a Entry> {
    let of_kind = || manifest.clients.iter().filter(|e| e.kind == kind.as_str());
    match arch {
        Some(arch) => of_kind().find(|e| e.arch == arch),
        None => of_kind()
            .find(|e| e.arch == kind.usual_arch())
            .or_else(|| of_kind().next()),
    }
}

/// The SHA-256 of a file, as lowercase hex, and its size.
pub fn hash_file(path: &Path) -> Result<(String, u64), Error> {
    let mut file = std::fs::File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((hex, size))
}

/// One build to publish.
#[derive(Debug, Clone)]
pub struct Publish {
    /// The built file.
    pub path: PathBuf,
    /// Its version; read from the file name when `None`.
    pub version: Option<String>,
    /// `release` or `debug`; `debug` when `None` and the file name says so, else `release`.
    pub build: Option<String>,
    /// Its architecture; read from the file name when `None`.
    pub arch: Option<String>,
}

impl Publish {
    pub fn file(path: impl Into<PathBuf>) -> Publish {
        Publish {
            path: path.into(),
            version: None,
            build: None,
            arch: None,
        }
    }
}

/// What a build will be published as, before anything is copied.
fn describe(item: &Publish) -> Result<Entry, Error> {
    let shown = item.path.display();
    let name = item
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| Error(format!("{shown} is not a file")))?;
    let kind = kind_of(&name).ok_or_else(|| {
        Error(format!(
            "{shown}: only .apk, .AppImage, .deb and .rpm files are published"
        ))
    })?;
    let version = match &item.version {
        Some(v) => v.trim().trim_start_matches('v').to_owned(),
        None => version_of(&name).ok_or_else(|| {
            Error(format!(
                "{shown}: the file name names no version; pass --version"
            ))
        })?,
    };
    if version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
    {
        return Err(Error(format!("{version:?} is not a version")));
    }
    let build = match &item.build {
        Some(b) if b == "release" || b == "debug" => b.clone(),
        Some(b) => {
            return Err(Error(format!(
                "build {b:?} must be \"release\" or \"debug\""
            )));
        }
        None if name.to_ascii_lowercase().contains("debug") => "debug".to_owned(),
        None => "release".to_owned(),
    };
    let arch = match &item.arch {
        Some(a) if !a.is_empty() && a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') => {
            a.clone()
        }
        Some(a) => return Err(Error(format!("{a:?} is not an architecture"))),
        None => arch_of(&name)
            .ok_or_else(|| {
                Error(format!(
                    "{shown}: the file name names no architecture; pass --arch"
                ))
            })?
            .to_owned(),
    };
    let suffix = if build == "debug" { "-debug" } else { "" };
    let filename = format!("TheVault_{version}_{arch}{suffix}.{}", kind.extension());
    Ok(Entry {
        platform: kind.platform().to_owned(),
        kind: kind.as_str().to_owned(),
        arch,
        version,
        build,
        size: 0,
        sha256: String::new(),
        filename,
        published: 0,
    })
}

/// Copy `items` into `dir`, creating it, and rewrite the manifest. Each build replaces the one
/// listed for its platform, kind and architecture, whose file is removed. Files are copied under
/// a temporary name and renamed into place, and the manifest is written last, the same way, so a
/// server reading the folder meanwhile sees the old manifest or the new one. `now` is the
/// publish time in milliseconds since the epoch.
pub fn publish(dir: &Path, items: &[Publish], now: i64) -> Result<Manifest, Error> {
    let mut entries = items.iter().map(describe).collect::<Result<Vec<_>, _>>()?;
    for (i, e) in entries.iter().enumerate() {
        if entries[..i]
            .iter()
            .any(|o| o.kind == e.kind && o.arch == e.arch)
        {
            return Err(Error(format!(
                "two {} builds for {} were given; publish one",
                e.kind, e.arch
            )));
        }
    }
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let mut manifest = read(dir)?;
    for (item, entry) in items.iter().zip(entries.iter_mut()) {
        let target = dir.join(&entry.filename);
        let partial = dir.join(format!(".{}.partial", entry.filename));
        std::fs::copy(&item.path, &partial).map_err(io_err(&item.path))?;
        let (sha256, size) = hash_file(&partial)?;
        std::fs::rename(&partial, &target).map_err(io_err(&target))?;
        entry.sha256 = sha256;
        entry.size = size;
        entry.published = now;
    }
    let mut stale = Vec::new();
    manifest.clients.retain(|old| {
        let replaced = entries
            .iter()
            .any(|e| e.kind == old.kind && e.arch == old.arch);
        if replaced && !entries.iter().any(|e| e.filename == old.filename) {
            stale.push(old.filename.clone());
        }
        !replaced
    });
    manifest.clients.extend(entries);
    manifest.clients.sort_by(|a, b| {
        let order = |e: &Entry| Kind::parse(&e.kind).map_or(usize::MAX, Kind::order);
        order(a).cmp(&order(b)).then_with(|| a.arch.cmp(&b.arch))
    });
    write_manifest(dir, &manifest)?;
    for name in stale {
        if is_safe_name(&name) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
    Ok(manifest)
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), Error> {
    let path = dir.join(MANIFEST);
    let partial = dir.join(format!(".{MANIFEST}.partial"));
    let mut text = serde_json::to_vec_pretty(manifest).map_err(|e| Error(e.to_string()))?;
    text.push(b'\n');
    let mut file = std::fs::File::create(&partial).map_err(io_err(&partial))?;
    file.write_all(&text).map_err(io_err(&partial))?;
    file.sync_all().map_err(io_err(&partial))?;
    drop(file);
    std::fs::rename(&partial, &path).map_err(io_err(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_kind_arch_and_version_from_file_names() {
        assert_eq!(kind_of("TheVault_0.1.0_aarch64.apk"), Some(Kind::Apk));
        assert_eq!(
            kind_of("TheVault_0.1.0_amd64.AppImage"),
            Some(Kind::AppImage)
        );
        assert_eq!(kind_of("x.appimage"), Some(Kind::AppImage));
        assert_eq!(kind_of("TheVault-0.1.0-1.x86_64.rpm"), Some(Kind::Rpm));
        assert_eq!(kind_of("SHA256SUMS"), None);
        assert_eq!(arch_of("app-arm64-debug.apk"), Some("aarch64"));
        assert_eq!(arch_of("TheVault_0.1.0_amd64.deb"), Some("x86_64"));
        assert_eq!(arch_of("TheVault-0.1.0-1.x86_64.rpm"), Some("x86_64"));
        assert_eq!(arch_of("app-universal-release.apk"), Some("universal"));
        assert_eq!(
            version_of("TheVault_0.1.0_aarch64.apk").as_deref(),
            Some("0.1.0")
        );
        assert_eq!(
            version_of("TheVault-0.12.3-1.x86_64.rpm").as_deref(),
            Some("0.12.3")
        );
        assert_eq!(version_of("app-arm64-debug.apk"), None);
    }

    #[test]
    fn refuses_names_that_could_leave_the_folder() {
        assert!(is_safe_name("TheVault_0.1.0_aarch64.apk"));
        for name in [
            "",
            "../x.apk",
            "a/b.apk",
            "a\\b.apk",
            ".hidden",
            "manifest.json",
            "a b.apk",
        ] {
            assert!(!is_safe_name(name), "{name:?}");
        }
    }

    #[test]
    fn a_missing_folder_is_an_empty_manifest() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("nope")).unwrap(), Manifest::default());
        assert!(find(&dir.path().join("nope"), "x.apk").unwrap().is_none());
    }

    #[test]
    fn publish_writes_the_manifest_with_size_and_sha256() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let clients = out.path().join("clients");
        let apk = build(src.path(), "app-arm64-debug.apk", b"apk bytes");
        let image = build(src.path(), "TheVault_0.1.0_amd64.AppImage", b"appimage");
        let manifest = publish(
            &clients,
            &[
                Publish {
                    version: Some("0.1.0".into()),
                    ..Publish::file(&apk)
                },
                Publish::file(&image),
            ],
            1_000,
        )
        .unwrap();
        assert_eq!(manifest, read(&clients).unwrap());
        let [a, l] = manifest.clients.as_slice() else {
            panic!("two entries: {manifest:?}");
        };
        assert_eq!(
            a,
            &Entry {
                platform: "android".into(),
                kind: "apk".into(),
                arch: "aarch64".into(),
                version: "0.1.0".into(),
                build: "debug".into(),
                size: 9,
                sha256: hash_file(&apk).unwrap().0,
                filename: "TheVault_0.1.0_aarch64-debug.apk".into(),
                published: 1_000,
            }
        );
        assert_eq!(l.kind, "appimage");
        assert_eq!(l.build, "release");
        assert_eq!(l.filename, "TheVault_0.1.0_x86_64.AppImage");
        assert_eq!(l.size, 8);
        assert_eq!(
            std::fs::read(clients.join(&l.filename)).unwrap(),
            b"appimage"
        );
        let (entry, path) = find(&clients, &a.filename).unwrap().unwrap();
        assert_eq!(entry, *a);
        assert_eq!(std::fs::read(path).unwrap(), b"apk bytes");
    }

    #[test]
    fn a_new_build_replaces_the_old_one_and_its_file() {
        let src = tempfile::tempdir().unwrap();
        let clients = tempfile::tempdir().unwrap();
        let old = build(src.path(), "TheVault_0.1.0_aarch64.apk", b"old");
        let image = build(src.path(), "TheVault_0.1.0_amd64.AppImage", b"img");
        publish(
            clients.path(),
            &[Publish::file(&old), Publish::file(&image)],
            1,
        )
        .unwrap();
        let new = build(src.path(), "TheVault_0.2.0_aarch64.apk", b"new");
        let manifest = publish(clients.path(), &[Publish::file(&new)], 2).unwrap();
        let names: Vec<&str> = manifest
            .clients
            .iter()
            .map(|e| e.filename.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "TheVault_0.2.0_aarch64.apk",
                "TheVault_0.1.0_x86_64.AppImage"
            ]
        );
        assert!(!clients.path().join("TheVault_0.1.0_aarch64.apk").exists());
        assert_eq!(latest(&manifest, Kind::Apk, None).unwrap().version, "0.2.0");
        assert!(latest(&manifest, Kind::Deb, None).is_none());
        assert!(
            find(clients.path(), "TheVault_0.1.0_aarch64.apk")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn refuses_what_it_cannot_describe() {
        let src = tempfile::tempdir().unwrap();
        let clients = tempfile::tempdir().unwrap();
        let sums = build(src.path(), "SHA256SUMS", b"");
        let unnamed = build(src.path(), "app-arm64-debug.apk", b"x");
        assert!(publish(clients.path(), &[Publish::file(&sums)], 1).is_err());
        let err = publish(clients.path(), &[Publish::file(&unnamed)], 1).unwrap_err();
        assert!(err.to_string().contains("--version"), "{err}");
        assert!(!clients.path().join(MANIFEST).exists());
    }

    #[test]
    fn hashes_files_as_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let path = build(dir.path(), "x.apk", b"abc");
        assert_eq!(
            hash_file(&path).unwrap(),
            (
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned(),
                3
            )
        );
    }
}
