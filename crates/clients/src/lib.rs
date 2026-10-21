//! The app builds a server hands out: the Android APK, the Linux AppImage, deb and rpm, and the
//! static Linux builds of the `vault` CLI (platform `cli`, kind `binary`, D27).
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
//!
//! A CLI build is a bare executable named `vault-<version>-linux-<arch>`, as `just cli-release`
//! writes it, and keeps that name in the folder.
//!
//! An AppImage published with its updater signature beside it, `<file>.AppImage.sig` as
//! `tauri signer` writes it, carries that signature in the manifest, and [`update_for`] offers
//! it to the desktop app's updater (`GET /api/clients/update/...`).

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
    /// `android`, `linux` or `cli`.
    pub platform: String,
    /// `apk`, `appimage`, `deb`, `rpm` or `binary`.
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
    /// The Tauri updater's signature of the file (the text of the `.sig` file `tauri signer`
    /// writes: minisign, base64), for an AppImage published with one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
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
    /// A static build of the `vault` CLI.
    Binary,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Apk,
        Kind::AppImage,
        Kind::Deb,
        Kind::Rpm,
        Kind::Binary,
    ];

    /// The name the manifest records: `apk`, `appimage`, `deb`, `rpm` or `binary`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Apk => "apk",
            Kind::AppImage => "appimage",
            Kind::Deb => "deb",
            Kind::Rpm => "rpm",
            Kind::Binary => "binary",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The platform it installs on.
    pub fn platform(self) -> &'static str {
        match self {
            Kind::Apk => "android",
            Kind::Binary => "cli",
            _ => "linux",
        }
    }

    /// The file extension, without the dot; empty for the CLI, which has none.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Apk => "apk",
            Kind::AppImage => "AppImage",
            Kind::Deb => "deb",
            Kind::Rpm => "rpm",
            Kind::Binary => "",
        }
    }

    /// The media type a download is served with.
    pub fn content_type(self) -> &'static str {
        match self {
            Kind::Apk => "application/vnd.android.package-archive",
            Kind::AppImage => "application/octet-stream",
            Kind::Deb => "application/vnd.debian.binary-package",
            Kind::Rpm => "application/x-rpm",
            Kind::Binary => "application/octet-stream",
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

/// The kind of build a file name ends in, by its extension, or [`Kind::Binary`] for a CLI build
/// named `vault-<version>-linux-<arch>`.
pub fn kind_of(filename: &str) -> Option<Kind> {
    if is_cli_name(filename) {
        return Some(Kind::Binary);
    }
    let ext = filename.rsplit_once('.')?.1.to_ascii_lowercase();
    Kind::ALL
        .into_iter()
        .find(|k| !k.extension().is_empty() && k.extension().eq_ignore_ascii_case(&ext))
}

/// Whether `filename` is a CLI build's: `vault-<version>-linux-<arch>`, with nothing after the
/// architecture.
fn is_cli_name(filename: &str) -> bool {
    let Some(rest) = filename.strip_prefix("vault-") else {
        return false;
    };
    let Some((version, arch)) = rest.split_once("-linux-") else {
        return false;
    };
    let arch = arch.strip_suffix("-debug").unwrap_or(arch);
    is_version_or_pre_release(version)
        && !arch.is_empty()
        && arch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
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
/// dot-separated numbers, such as `0.1.0` in `TheVault_0.1.0_aarch64.apk`, with the pre-release
/// after it when a `-` joins one that starts with a letter and another `_` or `-` ends it, such
/// as `0.1.13-alpha.1` in `TheVault_0.1.13-alpha.1_aarch64.apk` (ticket 139). The rpm's release
/// number after the version, `-1` in `TheVault-0.1.0-1.x86_64.rpm`, starts with a digit and is
/// no pre-release.
pub fn version_of(filename: &str) -> Option<String> {
    let mut start = 0;
    for part in filename.split(['_', '-']) {
        let end = start + part.len();
        if is_version(part) {
            let after = filename[end..].strip_prefix('-').and_then(|rest| {
                let pre = rest.split(['_', '-']).next()?;
                (pre.len() < rest.len() && is_pre_release(pre)).then_some(pre)
            });
            return Some(match after {
                Some(pre) => format!("{part}-{pre}"),
                None => part.to_owned(),
            });
        }
        start = end + 1;
    }
    None
}

/// Whether `s` is a pre-release a file name may carry, such as `alpha.1`: dot-separated letters
/// and digits, starting with a letter and holding a digit, and no architecture, so neither
/// `linux` in `vault-0.1.0-linux-x86_64` nor `arm64` in `app-0.1.0-arm64-debug.apk` is one.
fn is_pre_release(s: &str) -> bool {
    s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && s.bytes().any(|b| b.is_ascii_digit())
        && arch_of(s).is_none()
        && s.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Whether `s` is a version with or without a pre-release, `0.1.13` or `0.1.13-alpha.1`.
fn is_version_or_pre_release(s: &str) -> bool {
    match s.split_once('-') {
        Some((core, pre)) => is_version(core) && is_pre_release(pre),
        None => is_version(s),
    }
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

/// The AppImage the desktop app's updater should install, when there is one newer than
/// `current`: a release build for `target` (`linux`) and `arch` (`x86_64`, `aarch64`) published
/// with its updater signature. A build without a signature is never offered, since the updater
/// would refuse it. A pre-release is offered only by a server that runs a pre-release itself,
/// `server` being its version, so a stable server never moves its apps onto an alpha
/// ([`may_take`], ticket 139).
pub fn update_for<'a>(
    manifest: &'a Manifest,
    target: &str,
    arch: &str,
    current: &str,
    server: &str,
) -> Option<&'a Entry> {
    if target != Kind::AppImage.platform() {
        return None;
    }
    manifest
        .clients
        .iter()
        .filter(|e| e.kind == Kind::AppImage.as_str() && e.arch == arch && e.build == "release")
        .filter(|e| e.signature.is_some())
        .filter(|e| is_newer(&e.version, current) && may_take(&e.version, server))
        .max_by(|a, b| Version::parse(&a.version).cmp(&Version::parse(&b.version)))
}

/// Whether an app may update to version `candidate` from a server that runs version `server`: a
/// release always, a pre-release only when the server runs a pre-release too (ticket 139). A
/// candidate that does not parse never.
pub fn may_take(candidate: &str, server: &str) -> bool {
    match Version::parse(candidate) {
        Some(c) if c.pre.is_empty() => true,
        Some(_) => Version::parse(server).is_some_and(|s| !s.pre.is_empty()),
        None => false,
    }
}

/// Whether version `candidate` is newer than `current`, as semver orders them. A version that
/// does not parse is never newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// A semantic version, compared by precedence: build metadata does not count, and a pre-release
/// sorts before its release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}

impl Version {
    /// `1.2.3`, `v1.2.3`, `1.2.3-rc.1` or `1.2.3+build`.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split_once('+').map_or(s, |(v, _)| v);
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        };
        let nums: Vec<&str> = core.split('.').collect();
        let [major, minor, patch] = nums.as_slice() else {
            return None;
        };
        let num = |p: &str| -> Option<u64> {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let core = [num(major)?, num(minor)?, num(patch)?];
        let pre: Vec<String> = match pre {
            Some(pre) => pre.split('.').map(str::to_owned).collect(),
            None => Vec::new(),
        };
        if pre.iter().any(String::is_empty) {
            return None;
        }
        Some(Version { core, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            })
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` (RFC 3339, UTC) for a time in milliseconds since the epoch, as the
/// updater's `pub_date` carries it.
pub fn rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
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
            "{shown}: only .apk, .AppImage, .deb and .rpm files and vault-<version>-linux-<arch> CLI builds are published"
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
    let filename = match kind {
        Kind::Binary => format!("vault-{version}-linux-{arch}{suffix}"),
        _ => format!("TheVault_{version}_{arch}{suffix}.{}", kind.extension()),
    };
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
        signature: None,
    })
}

/// The updater signature beside an AppImage, `<path>.sig`, when there is one.
fn signature_beside(path: &Path) -> Result<Option<String>, Error> {
    let mut sig = path.as_os_str().to_owned();
    sig.push(".sig");
    let sig = PathBuf::from(sig);
    match std::fs::read_to_string(&sig) {
        Ok(text) if !text.trim().is_empty() => Ok(Some(text.trim().to_owned())),
        Ok(_) => Err(Error(format!("{}: the signature is empty", sig.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(&sig)(e)),
    }
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
        if entry.kind == Kind::AppImage.as_str() {
            entry.signature = signature_beside(&item.path)?;
        }
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
    fn reads_a_pre_release_from_file_names() {
        for (name, version, kind) in [
            (
                "TheVault_0.1.13-alpha.1_aarch64.apk",
                "0.1.13-alpha.1",
                Kind::Apk,
            ),
            (
                "TheVault_0.1.13-alpha.1_amd64.AppImage",
                "0.1.13-alpha.1",
                Kind::AppImage,
            ),
            (
                "TheVault_0.1.13-alpha.12_amd64.deb",
                "0.1.13-alpha.12",
                Kind::Deb,
            ),
            (
                "TheVault-0.1.13-alpha.1-1.x86_64.rpm",
                "0.1.13-alpha.1",
                Kind::Rpm,
            ),
            (
                "vault-0.1.13-alpha.1-linux-x86_64",
                "0.1.13-alpha.1",
                Kind::Binary,
            ),
            (
                "vault-0.1.13-alpha.1-linux-aarch64",
                "0.1.13-alpha.1",
                Kind::Binary,
            ),
            ("TheVault-0.1.13-1.x86_64.rpm", "0.1.13", Kind::Rpm),
        ] {
            assert_eq!(version_of(name).as_deref(), Some(version), "{name}");
            assert_eq!(kind_of(name), Some(kind), "{name}");
        }
        assert_eq!(
            arch_of("vault-0.1.13-alpha.1-linux-aarch64"),
            Some("aarch64")
        );
        assert_eq!(kind_of("vault-0.1.13-1-linux-x86_64"), None);
        assert_eq!(
            version_of("vault-0.3.0-linux-x86_64").as_deref(),
            Some("0.3.0")
        );
        assert_eq!(
            version_of("app-0.3.0-arm64-debug.apk").as_deref(),
            Some("0.3.0")
        );
    }

    #[test]
    fn a_pre_release_appimage_is_offered_only_by_a_pre_release_server() {
        let entry = |version: &str| Entry {
            platform: "linux".into(),
            kind: "appimage".into(),
            arch: "x86_64".into(),
            version: version.into(),
            build: "release".into(),
            size: 1,
            sha256: "x".into(),
            filename: format!("TheVault_{version}_amd64.AppImage"),
            published: 0,
            signature: Some("sig".into()),
        };
        let manifest = Manifest {
            clients: vec![entry("0.1.13"), entry("0.1.14-alpha.1")],
        };
        let offer = |current, server| {
            update_for(&manifest, "linux", "x86_64", current, server).map(|e| e.version.as_str())
        };
        // A stable server offers its release and never the alpha beside it.
        assert_eq!(offer("0.1.12", "0.1.13"), Some("0.1.13"));
        assert_eq!(offer("0.1.13", "0.1.13"), None);
        // A pre-release server offers the alpha.
        assert_eq!(offer("0.1.12", "0.1.14-alpha.1"), Some("0.1.14-alpha.1"));
        assert_eq!(offer("0.1.13", "0.1.14-alpha.1"), Some("0.1.14-alpha.1"));
        assert!(may_take("0.1.13", "0.1.12"));
        assert!(may_take("0.1.13", "not a version"));
        assert!(!may_take("0.1.13-alpha.1", "0.1.12"));
        assert!(!may_take("0.1.13-alpha.1", ""));
        assert!(may_take("0.1.13-alpha.2", "0.1.13-alpha.1"));
        assert!(!may_take("garbage", "0.1.13-alpha.1"));
    }

    #[test]
    fn a_vault_cli_build_publishes_as_a_cli_binary() {
        assert_eq!(kind_of("vault-0.1.11-linux-x86_64"), Some(Kind::Binary));
        assert_eq!(kind_of("vault-0.1.11-linux-aarch64"), Some(Kind::Binary));
        assert_eq!(kind_of("vault-server"), None);
        assert_eq!(kind_of("vault-0.1.11-linux-x86_64.deb"), Some(Kind::Deb));
        let src = tempfile::tempdir().unwrap();
        let clients = tempfile::tempdir().unwrap();
        let amd64 = build(src.path(), "vault-0.2.0-linux-x86_64", b"amd64");
        let arm64 = build(src.path(), "vault-0.2.0-linux-aarch64", b"arm64");
        let image = build(src.path(), "TheVault_0.2.0_amd64.AppImage", b"img");
        let manifest = publish(
            clients.path(),
            &[
                Publish::file(&arm64),
                Publish::file(&image),
                Publish::file(&amd64),
            ],
            5,
        )
        .unwrap();
        let names: Vec<(&str, &str, &str)> = manifest
            .clients
            .iter()
            .map(|e| (e.platform.as_str(), e.kind.as_str(), e.filename.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("linux", "appimage", "TheVault_0.2.0_x86_64.AppImage"),
                ("cli", "binary", "vault-0.2.0-linux-aarch64"),
                ("cli", "binary", "vault-0.2.0-linux-x86_64"),
            ]
        );
        let cli = latest(&manifest, Kind::Binary, Some("aarch64")).unwrap();
        assert_eq!(
            (cli.version.as_str(), cli.build.as_str()),
            ("0.2.0", "release")
        );
        assert_eq!(cli.size, 5);
        assert_eq!(
            latest(&manifest, Kind::Binary, None).unwrap().arch,
            "x86_64"
        );
        // A newer build replaces the one for its architecture alone.
        let newer = build(src.path(), "vault-0.3.0-linux-x86_64", b"newer");
        let manifest = publish(clients.path(), &[Publish::file(&newer)], 6).unwrap();
        assert_eq!(
            latest(&manifest, Kind::Binary, Some("x86_64"))
                .unwrap()
                .filename,
            "vault-0.3.0-linux-x86_64"
        );
        assert!(!clients.path().join("vault-0.2.0-linux-x86_64").exists());
        assert!(latest(&manifest, Kind::Binary, Some("aarch64")).is_some());
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
                signature: None,
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
    fn an_appimage_carries_the_signature_published_beside_it() {
        let src = tempfile::tempdir().unwrap();
        let clients = tempfile::tempdir().unwrap();
        let image = build(src.path(), "TheVault_0.2.0_amd64.AppImage", b"img");
        build(
            src.path(),
            "TheVault_0.2.0_amd64.AppImage.sig",
            b"c2lnbmF0dXJl\n",
        );
        let unsigned = build(src.path(), "TheVault_0.2.0_amd64.deb", b"deb");
        let manifest = publish(
            clients.path(),
            &[Publish::file(&image), Publish::file(&unsigned)],
            1_760_000_000_000,
        )
        .unwrap();
        let image = latest(&manifest, Kind::AppImage, None).unwrap();
        assert_eq!(image.signature.as_deref(), Some("c2lnbmF0dXJl"));
        assert_eq!(latest(&manifest, Kind::Deb, None).unwrap().signature, None);
        let text = std::fs::read_to_string(clients.path().join(MANIFEST)).unwrap();
        assert_eq!(text.matches("\"signature\"").count(), 1, "{text}");

        let version = |current| update_for(&manifest, "linux", "x86_64", current, "0.2.0");
        assert_eq!(version("0.1.0").unwrap().version, "0.2.0");
        assert!(version("0.2.0").is_none());
        assert!(version("0.3.0").is_none());
        assert!(update_for(&manifest, "linux", "aarch64", "0.1.0", "0.2.0").is_none());
        assert!(update_for(&manifest, "windows", "x86_64", "0.1.0", "0.2.0").is_none());
    }

    #[test]
    fn an_unsigned_or_debug_appimage_is_never_an_update() {
        let entry = |build: &str, signature: Option<&str>| Entry {
            platform: "linux".into(),
            kind: "appimage".into(),
            arch: "x86_64".into(),
            version: "1.0.0".into(),
            build: build.into(),
            size: 1,
            sha256: "x".into(),
            filename: "TheVault_1.0.0_x86_64.AppImage".into(),
            published: 0,
            signature: signature.map(str::to_owned),
        };
        for e in [entry("release", None), entry("debug", Some("sig"))] {
            let manifest = Manifest { clients: vec![e] };
            assert!(update_for(&manifest, "linux", "x86_64", "0.1.0", "1.0.0").is_none());
        }
    }

    #[test]
    fn versions_compare_as_semver() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("v1.0.0", "1.0.0-rc.1"));
        assert!(is_newer("1.0.0-rc.10", "1.0.0-rc.2"));
        assert!(is_newer("1.0.0-beta", "1.0.0-alpha"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0+build.7", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("1.0", "0.1.0"));
        assert!(!is_newer("latest", "0.1.0"));
        assert!(!is_newer("2.0.0", "nonsense"));
    }

    #[test]
    fn formats_rfc3339_times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_791_796_507_999), "2026-10-12T09:15:07Z");
        assert_eq!(rfc3339(-1000), "1969-12-31T23:59:59Z");
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
