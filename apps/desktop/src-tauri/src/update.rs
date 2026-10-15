//! The update check (D27): the app asks GitHub Releases for the latest release and, when it is
//! newer than the running app, the window offers to download it.
//!
//! The check runs a minute after start and once a day after that, while the "Check for updates"
//! setting is on. `THEVAULT_UPDATE_URL` replaces the GitHub address (the S6 tests point it at a
//! local server); set to `off`, it turns the check off. The repository defaults to
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
    /// The release's page, where the user downloads it.
    pub url: String,
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

/// Asks `url` for the latest release, and answers it when it is newer than `current`. It blocks
/// while it waits.
pub fn check(url: &str, current: &str) -> Result<Option<Release>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("thevault-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
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
                url: "https://github.com/o/r/releases/tag/v0.2.0".into()
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
}
