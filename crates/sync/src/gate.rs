//! The version gate: an app updates itself before it syncs with a server newer than itself (D27).
//!
//! The server is upgraded first and every app catches up before it reads anything new. Before
//! each round an app compares the server's version (`server_version` in `GET /api/sync/info`)
//! with its own. When the server is newer and the server publishes a build of this app at least
//! as new as the server, the app pauses sync, pushing and pulling both, and updates itself; the
//! new version syncs once it starts. When the server publishes no such build, sync goes on and
//! the app warns that it is older than the server. An update that fails keeps sync paused and is
//! tried again; after [`MAX_FAILURES`] failures for one version sync goes on with a warning.
//!
//! Both apps use this module: the Android core before each round of its foreground loop and
//! background worker, and the desktop's sync loop. Each finds the published build its own way,
//! Android in `GET /api/clients` ([`apk_for`]), the desktop with the Tauri updater.

use std::cmp::Ordering;
use std::sync::{Mutex, PoisonError};

use serde_json::Value as Json;

/// How many failed updates to one version pause sync before it goes on with a warning.
pub const MAX_FAILURES: u32 = 3;

/// Compares two versions as semver does: `major.minor.patch` by number, and a pre-release
/// (`1.0.0-rc.1`) before its release. Build metadata (`+…`) does not count. `None` when either
/// is not a version.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (x, xp) = parse(a)?;
    let (y, yp) = parse(b)?;
    Some(x.cmp(&y).then_with(|| match (xp, yp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(p), Some(q)) => compare_pre(p, q),
    }))
}

fn parse(v: &str) -> Option<([u64; 3], Option<&str>)> {
    let v = v.trim();
    let core = v.strip_prefix('v').unwrap_or(v);
    let core = core.split('+').next().unwrap_or(core);
    let (nums, pre) = match core.split_once('-') {
        Some((n, p)) if !p.is_empty() => (n, Some(p)),
        Some((n, _)) => (n, None),
        None => (core, None),
    };
    let mut parts = nums.split('.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some((out, pre))
}

fn compare_pre(a: &str, b: &str) -> Ordering {
    let (x, y): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
    for (p, q) in x.iter().zip(&y) {
        let c = match (p.parse::<u64>(), q.parse::<u64>()) {
            (Ok(n), Ok(m)) => n.cmp(&m),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => p.cmp(q),
        };
        if c != Ordering::Equal {
            return c;
        }
    }
    x.len().cmp(&y.len())
}

/// Failed updates to one version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Failures {
    /// The version the updates tried to install.
    pub version: String,
    pub count: u32,
    /// Why the last one failed.
    pub message: Option<String>,
}

impl Failures {
    /// How many updates to `version` failed.
    pub fn of(&self, version: &str) -> u32 {
        if compare(&self.version, version) == Some(Ordering::Equal) {
            self.count
        } else {
            0
        }
    }
}

/// What the gate decided for one round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Sync as usual: the server is not newer than the app.
    Sync,
    /// Sync, but tell the user `message`: the server is newer and the app cannot update to it.
    Warn { message: String },
    /// Do not sync: update the app to `version` first. `message` says so.
    Pause { version: String, message: String },
}

impl Verdict {
    /// What the app shows, when it shows anything.
    pub fn message(&self) -> Option<&str> {
        match self {
            Verdict::Sync => None,
            Verdict::Warn { message } | Verdict::Pause { message, .. } => Some(message),
        }
    }
}

/// The gate's rule. `installed` is the app's version, `server` the server's, `published` the
/// newest build of this app the server publishes for it, and `failures` the updates that failed.
pub fn decide(
    installed: &str,
    server: &str,
    published: Option<&str>,
    failures: &Failures,
) -> Verdict {
    if compare(server, installed) != Some(Ordering::Greater) {
        return Verdict::Sync;
    }
    let usable = published.filter(|p| compare(p, server).is_some_and(|c| c != Ordering::Less));
    let Some(version) = usable else {
        return Verdict::Warn {
            message: format!(
                "The server runs TheVault {server}, newer than this app's {installed}, and \
                 publishes no matching app build. Sync goes on; publish the app build on the \
                 server (just publish-clients) so the app can update itself."
            ),
        };
    };
    let failed = failures.of(version);
    let why = failures
        .message
        .as_deref()
        .filter(|_| failed > 0)
        .map(|m| format!(" The last try failed: {m}"))
        .unwrap_or_default();
    if failed >= MAX_FAILURES {
        return Verdict::Warn {
            message: format!(
                "The server runs TheVault {server}, newer than this app's {installed}, and \
                 updating to {version} failed {failed} times, so sync goes on without it.{why}"
            ),
        };
    }
    Verdict::Pause {
        version: version.to_owned(),
        message: format!("Updating TheVault to {version} before syncing.{why}"),
    }
}

/// The newest APK in a `GET /api/clients` answer for an Android app of `build` (`release` or
/// `debug`) on a phone whose architectures, in the server's names, are `arches`; a universal APK
/// counts for any phone.
pub fn apk_for(manifest: &Json, arches: &[String], build: &str) -> Option<String> {
    manifest
        .get("clients")?
        .as_array()?
        .iter()
        .filter(|e| e.get("kind").and_then(Json::as_str) == Some("apk"))
        .filter(|e| e.get("build").and_then(Json::as_str).unwrap_or("release") == build)
        .filter(|e| {
            e.get("arch")
                .and_then(Json::as_str)
                .is_some_and(|a| a == "universal" || arches.iter().any(|x| x == a))
        })
        .filter_map(|e| e.get("version").and_then(Json::as_str))
        .filter(|v| parse(v).is_some())
        .max_by(|a, b| compare(a, b).unwrap_or(Ordering::Equal))
        .map(str::to_owned)
}

/// The gate's memory between rounds: the failed updates, and the last verdict for the app to
/// show. An app holds one for as long as it runs.
#[derive(Debug, Default)]
pub struct UpdateGate {
    failures: Mutex<Failures>,
    verdict: Mutex<Option<Verdict>>,
}

impl UpdateGate {
    pub fn new() -> UpdateGate {
        UpdateGate::default()
    }

    /// Decide this round, and remember the verdict.
    pub fn judge(&self, installed: &str, server: &str, published: Option<&str>) -> Verdict {
        let failures = self.failures();
        let verdict = decide(installed, server, published, &failures);
        *self.verdict.lock().unwrap_or_else(PoisonError::into_inner) = Some(verdict.clone());
        verdict
    }

    /// An update to `version` failed with `message`. Answers how many have failed for it.
    pub fn failed(&self, version: &str, message: &str) -> u32 {
        let mut f = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let count = f.of(version) + 1;
        *f = Failures {
            version: version.to_owned(),
            count,
            message: Some(message.to_owned()),
        };
        count
    }

    /// Replace the failures, as an app that keeps them across restarts does.
    pub fn set_failures(&self, failures: Failures) {
        *self.failures.lock().unwrap_or_else(PoisonError::into_inner) = failures;
    }

    pub fn failures(&self) -> Failures {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The last verdict, or `None` before the first.
    pub fn verdict(&self) -> Option<Verdict> {
        self.verdict
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Forget the last verdict, as when the app is unpaired.
    pub fn clear(&self) {
        *self.verdict.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// How long to wait before trying an update again after `failures` failures: a minute, then four
/// times longer each time, at most an hour.
pub fn backoff(failures: u32) -> std::time::Duration {
    let minutes = 4u64.saturating_pow(failures.saturating_sub(1)).min(60);
    std::time::Duration::from_secs(60 * minutes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn none() -> Failures {
        Failures::default()
    }

    #[test]
    fn versions_compare_as_semver() {
        assert_eq!(compare("0.10.0", "0.9.0"), Some(Ordering::Greater));
        assert_eq!(compare("1.0.0+7", "v1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Some(Ordering::Less));
        assert_eq!(compare("1.0.0-rc.2", "1.0.0-rc.10"), Some(Ordering::Less));
        assert_eq!(compare("1.0", "1.0.0"), None);
        assert_eq!(compare("fake", "1.0.0"), None);
    }

    #[test]
    fn the_same_or_an_older_server_syncs() {
        assert_eq!(
            decide("0.1.8", "0.1.8", Some("0.1.9"), &none()),
            Verdict::Sync
        );
        assert_eq!(decide("0.1.8", "0.1.7", None, &none()), Verdict::Sync);
        // A server that names no version is not newer.
        assert_eq!(decide("0.1.8", "fake", None, &none()), Verdict::Sync);
    }

    #[test]
    fn a_newer_server_with_a_build_pauses_for_the_update() {
        let v = decide("0.1.7", "0.1.8", Some("0.1.8"), &none());
        assert_eq!(
            v,
            Verdict::Pause {
                version: "0.1.8".into(),
                message: "Updating TheVault to 0.1.8 before syncing.".into()
            }
        );
        // A build newer than the server will do too.
        assert!(matches!(
            decide("0.1.7", "0.1.8", Some("0.1.9"), &none()),
            Verdict::Pause { version, .. } if version == "0.1.9"
        ));
    }

    #[test]
    fn a_newer_server_with_no_build_or_an_older_one_warns_and_syncs() {
        for published in [None, Some("0.1.7"), Some("garbage")] {
            let v = decide("0.1.7", "0.1.8", published, &none());
            let Verdict::Warn { message } = v else {
                panic!("{published:?}: {v:?}")
            };
            assert!(message.contains("no matching app build"), "{message}");
        }
    }

    #[test]
    fn three_failed_updates_resume_sync_with_a_warning() {
        let gate = UpdateGate::new();
        for n in 1..MAX_FAILURES {
            assert_eq!(gate.failed("0.1.8", "the download was short"), n);
            let v = gate.judge("0.1.7", "0.1.8", Some("0.1.8"));
            let Verdict::Pause { message, .. } = v else {
                panic!("{v:?}")
            };
            assert!(message.contains("the download was short"), "{message}");
        }
        assert_eq!(gate.failed("0.1.8", "the download was short"), MAX_FAILURES);
        let v = gate.judge("0.1.7", "0.1.8", Some("0.1.8"));
        let Verdict::Warn { message } = &v else {
            panic!("{v:?}")
        };
        assert!(message.contains("failed 3 times"), "{message}");
        assert_eq!(gate.verdict(), Some(v));
        // A newer build starts the count again.
        assert!(matches!(
            gate.judge("0.1.7", "0.1.8", Some("0.1.9")),
            Verdict::Pause { .. }
        ));
    }

    #[test]
    fn the_apk_for_a_phone_is_the_newest_of_its_build_and_arch() {
        let m = json!({ "clients": [
            { "kind": "apk", "arch": "aarch64", "build": "release", "version": "0.1.8" },
            { "kind": "apk", "arch": "aarch64", "build": "release", "version": "0.1.10" },
            { "kind": "apk", "arch": "aarch64", "build": "debug", "version": "0.2.0" },
            { "kind": "apk", "arch": "x86_64", "build": "release", "version": "0.3.0" },
            { "kind": "appimage", "arch": "aarch64", "version": "0.4.0" },
        ]});
        let arm = ["aarch64".to_owned()];
        assert_eq!(apk_for(&m, &arm, "release").as_deref(), Some("0.1.10"));
        assert_eq!(apk_for(&m, &arm, "debug").as_deref(), Some("0.2.0"));
        assert_eq!(apk_for(&m, &["armv7".to_owned()], "release"), None);
        let universal = json!({ "clients": [
            { "kind": "apk", "arch": "universal", "version": "0.5.0" },
        ]});
        assert_eq!(
            apk_for(&universal, &["armv7".to_owned()], "release").as_deref(),
            Some("0.5.0")
        );
    }

    #[test]
    fn retries_back_off() {
        assert_eq!(backoff(1).as_secs(), 60);
        assert_eq!(backoff(2).as_secs(), 240);
        assert_eq!(backoff(9).as_secs(), 3600);
    }
}
