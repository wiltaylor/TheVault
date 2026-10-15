//! The update check (D27): the app asks the server it is paired with for the AppImage it hands
//! out, at `/api/clients`, and, when the server publishes none or cannot be reached, asks GitHub
//! Releases for the latest release. When the version found is newer than the running app, the
//! window offers to download it: the server's AppImage itself, or the GitHub release page.
//!
//! The check runs soon after start and once a day after that, while the "Check for updates"
//! setting is on. `THEVAULT_UPDATE_URL` replaces the GitHub address (the S6 tests point it at a
//! local server); set to `off`, it turns the whole check off. The repository defaults to
//! [`DEFAULT_REPO`], and a build can name another with `THEVAULT_RELEASES_REPO`.

use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};

/// The GitHub repository whose releases the app checks, `owner/name`.
const DEFAULT_REPO: &str = "wiltaylor/thevault";

/// How long the check waits for GitHub.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A release newer than the running app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    /// The release's version, without a leading `v`.
    pub version: String,
    /// Where the user downloads it: the server's AppImage, or the GitHub release page.
    pub url: String,
    /// `server` or `github`.
    pub source: &'static str,
}

/// One build in the server's `/api/clients` list, as far as the check reads it.
#[derive(Debug, Deserialize)]
struct ServerBuild {
    kind: String,
    arch: String,
    version: String,
    #[serde(default)]
    url: Option<String>,
    path: String,
}

/// The server's `/api/clients` answer.
#[derive(Debug, Deserialize)]
struct ServerBuilds {
    #[serde(default)]
    clients: Vec<ServerBuild>,
}

/// What the GitHub Releases API says about one release, as far as the check reads it.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The version a release tag names: `v1.2.3` or `1.2.3`, with an optional pre-release part.
pub fn tag_version(tag: &str) -> Option<Version> {
    let tag = tag.trim();
    let tag = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    Version::parse(tag).ok()
}

/// The version `tag` names when it is newer than `current`, the running app's version.
///
/// Versions compare as semver does: a pre-release sorts before its release, and build metadata
/// does not count. A tag that is not a version is never newer.
pub fn newer(current: &str, tag: &str) -> Option<Version> {
    let current = tag_version(current)?;
    let latest = tag_version(tag)?;
    (latest.cmp_precedence(&current).is_gt()).then_some(latest)
}

/// Where the check looks, or `None` when it is turned off.
pub fn endpoint() -> Option<String> {
    match std::env::var("THEVAULT_UPDATE_URL") {
        Ok(url) if url.trim().eq_ignore_ascii_case("off") || url.trim().is_empty() => None,
        Ok(url) => Some(url.trim().to_owned()),
        Err(_) => {
            let repo = option_env!("THEVAULT_RELEASES_REPO").unwrap_or(DEFAULT_REPO);
            Some(format!(
                "https://api.github.com/repos/{repo}/releases/latest"
            ))
        }
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("thevault-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Checks the paired server first, and GitHub at `github` only when the server publishes no
/// AppImage for this machine or cannot be reached. It blocks while it waits.
pub fn check_preferring_server(
    server: Option<&str>,
    github: &str,
    current: &str,
) -> Result<Option<Release>, String> {
    let from_server = match server {
        Some(server) => check_server(server, current, std::env::consts::ARCH),
        None => Ok(None),
    };
    prefer_server(from_server, || check(github, current))
}

/// The server's answer when it gave one, else what `github` finds. `server` is `Ok(None)` when
/// the server publishes no AppImage, and `Ok(Some(None))` when it does and it is not newer.
fn prefer_server(
    server: Result<Option<Option<Release>>, String>,
    github: impl FnOnce() -> Result<Option<Release>, String>,
) -> Result<Option<Release>, String> {
    match server {
        Ok(Some(found)) => Ok(found),
        Ok(None) => github(),
        Err(e) => {
            eprintln!("thevault: update check: {e}; asking GitHub");
            github()
        }
    }
}

/// Asks the server at `server` for the AppImage it hands out for `arch`: `None` when it hands
/// out none, else the release when it is newer than `current`.
pub fn check_server(
    server: &str,
    current: &str,
    arch: &str,
) -> Result<Option<Option<Release>>, String> {
    let base = server.trim_end_matches('/');
    let url = format!("{base}/api/clients");
    let res = client()?
        .get(&url)
        .header("accept", "application/json")
        .send()
        .map_err(|e| format!("cannot reach {url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("{url} answered {}", res.status()));
    }
    let builds: ServerBuilds = res
        .json()
        .map_err(|e| format!("{url} sent no list of builds: {e}"))?;
    Ok(server_release(base, &builds, current, arch))
}

/// The AppImage in `builds` for `arch`, as a release when it is newer than `current`.
fn server_release(
    base: &str,
    builds: &ServerBuilds,
    current: &str,
    arch: &str,
) -> Option<Option<Release>> {
    let build = builds
        .clients
        .iter()
        .filter(|b| b.kind == "appimage" && b.arch == arch)
        .filter_map(|b| tag_version(&b.version).map(|v| (v, b)))
        .max_by(|a, b| a.0.cmp_precedence(&b.0))?
        .1;
    let url = build
        .url
        .clone()
        .unwrap_or_else(|| format!("{base}{}", build.path));
    Some(newer(current, &build.version).map(|version| Release {
        version: version.to_string(),
        url,
        source: "server",
    }))
}

/// Asks `url` for the latest release, and answers it when it is newer than `current`. It blocks
/// while it waits.
pub fn check(url: &str, current: &str) -> Result<Option<Release>, String> {
    let res = client()?
        .get(url)
        .header("accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("cannot reach {url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("{url} answered {}", res.status()));
    }
    let release: GitHubRelease = res
        .json()
        .map_err(|e| format!("{url} sent no release: {e}"))?;
    Ok(newer_release(current, &release))
}

fn newer_release(current: &str, release: &GitHubRelease) -> Option<Release> {
    if release.draft || release.prerelease {
        return None;
    }
    newer(current, &release.tag_name).map(|version| Release {
        version: version.to_string(),
        url: release.html_url.clone(),
        source: "github",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_version_is_newer() {
        assert_eq!(newer("0.1.0", "v0.2.0"), Some(Version::new(0, 2, 0)));
        assert_eq!(newer("0.1.0", "0.1.1"), Some(Version::new(0, 1, 1)));
        assert_eq!(newer("0.9.9", "v1.0.0"), Some(Version::new(1, 0, 0)));
        // Numbers compare as numbers, not as text.
        assert_eq!(newer("0.9.0", "v0.10.0"), Some(Version::new(0, 10, 0)));
    }

    #[test]
    fn the_same_or_a_lower_version_is_not_newer() {
        assert_eq!(newer("0.2.0", "v0.2.0"), None);
        assert_eq!(newer("0.2.0", "V0.2.0"), None);
        assert_eq!(newer("0.2.0", "v0.1.9"), None);
        assert_eq!(newer("1.0.0", "v0.10.0"), None);
    }

    #[test]
    fn a_pre_release_sorts_before_its_release() {
        assert_eq!(newer("1.0.0-rc.1", "v1.0.0"), Some(Version::new(1, 0, 0)));
        assert_eq!(newer("1.0.0", "v1.0.0-rc.2"), None);
        assert!(newer("1.0.0-rc.1", "v1.0.0-rc.2").is_some());
        assert!(newer("1.0.0-rc.2", "v1.0.0-rc.10").is_some());
    }

    #[test]
    fn build_metadata_does_not_count() {
        assert_eq!(newer("1.0.0", "v1.0.0+build.7"), None);
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_never_newer() {
        for tag in ["", "latest", "v1", "v1.2", "release-2026-10", "vv1.0.0"] {
            assert_eq!(newer("0.1.0", tag), None, "{tag:?}");
        }
    }

    #[test]
    fn drafts_and_pre_releases_are_not_offered() {
        let release = |tag: &str, draft, prerelease| GitHubRelease {
            tag_name: tag.to_owned(),
            html_url: format!("https://github.com/o/r/releases/tag/{tag}"),
            draft,
            prerelease,
        };
        assert_eq!(
            newer_release("0.1.0", &release("v0.2.0", false, false)),
            Some(Release {
                version: "0.2.0".into(),
                url: "https://github.com/o/r/releases/tag/v0.2.0".into(),
                source: "github",
            })
        );
        assert_eq!(
            newer_release("0.1.0", &release("v0.2.0", true, false)),
            None
        );
        assert_eq!(
            newer_release("0.1.0", &release("v0.2.0", false, true)),
            None
        );
    }

    fn builds(list: serde_json::Value) -> ServerBuilds {
        serde_json::from_value(serde_json::json!({ "clients": list })).unwrap()
    }

    fn appimage(version: &str, arch: &str) -> serde_json::Value {
        let name = format!("TheVault_{version}_{arch}.AppImage");
        serde_json::json!({
            "platform": "linux", "kind": "appimage", "arch": arch, "version": version,
            "build": "release", "size": 3, "sha256": "x", "filename": name,
            "path": format!("/api/clients/{name}"),
        })
    }

    #[test]
    fn the_server_offers_its_appimage_for_this_architecture() {
        let list = builds(serde_json::json!([
            { "platform": "android", "kind": "apk", "arch": "aarch64", "version": "9.0.0",
              "path": "/api/clients/a.apk" },
            appimage("0.3.0", "aarch64"),
            appimage("0.2.0", "x86_64"),
        ]));
        assert_eq!(
            server_release("http://vault:8080", &list, "0.1.0", "x86_64"),
            Some(Some(Release {
                version: "0.2.0".into(),
                url: "http://vault:8080/api/clients/TheVault_0.2.0_x86_64.AppImage".into(),
                source: "server",
            }))
        );
        // The same version: the server has the AppImage, and nothing is newer.
        assert_eq!(server_release("http://vault", &list, "0.2.0", "x86_64"), Some(None));
        // No AppImage for this machine, or none at all.
        assert_eq!(server_release("http://vault", &list, "0.1.0", "riscv64"), None);
        let empty = builds(serde_json::json!([]));
        assert_eq!(server_release("http://vault", &empty, "0.1.0", "x86_64"), None);
    }

    #[test]
    fn the_update_check_prefers_the_servers_manifest() {
        let github = Release {
            version: "9.0.0".into(),
            url: "https://github.com/o/r/releases/tag/v9.0.0".into(),
            source: "github",
        };
        let server = Release {
            version: "0.2.0".into(),
            url: "http://vault/api/clients/TheVault_0.2.0_x86_64.AppImage".into(),
            source: "server",
        };
        let never = || -> Result<Option<Release>, String> { panic!("GitHub was asked") };
        assert_eq!(
            prefer_server(Ok(Some(Some(server.clone()))), never),
            Ok(Some(server))
        );
        // The server's AppImage is the running version: nothing to offer, and GitHub is not asked.
        assert_eq!(prefer_server(Ok(Some(None)), never), Ok(None));
        // No AppImage on the server, or the server cannot be reached: GitHub.
        let ask = || Ok(Some(github.clone()));
        assert_eq!(prefer_server(Ok(None), ask), Ok(Some(github.clone())));
        assert_eq!(prefer_server(Err("down".into()), ask), Ok(Some(github.clone())));
    }

    #[test]
    fn asks_the_server_over_http() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let body = serde_json::json!({ "clients": [appimage("0.5.0", "x86_64")] }).to_string();
        let served = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
            }
            write!(
                conn,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        });
        let found = check_server(&format!("{base}/"), "0.1.0", "x86_64").unwrap();
        assert_eq!(
            found,
            Some(Some(Release {
                version: "0.5.0".into(),
                url: format!("{base}/api/clients/TheVault_0.5.0_x86_64.AppImage"),
                source: "server",
            }))
        );
        assert!(served.join().unwrap().starts_with("GET /api/clients HTTP/1.1"));
    }
}
