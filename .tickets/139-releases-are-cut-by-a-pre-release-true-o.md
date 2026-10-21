---
id: '139'
title: 'Releases are cut by a pre-release: true or release: true commit trailer on
  main, with proper pre-release versions for the image, Linux packages, Android versionCode
  and updaters'
blocked_by: []
status: todo
verified: unknown
---

# Releases are cut by a pre-release: true or release: true commit trailer on main, with proper pre-release versions for the image, Linux packages, Android versionCode and updaters

## Worklog

### 2026-10-21T02:42:17Z — wil

Spec (the ticket body was lost when filing; this entry is the spec). The user decided (2026-10-21): releases are cut by a commit trailer, like their other repos (see /home/wil/dev/wharf/.github/workflows/release.yml and /home/wil/dev/wcl/.github/workflows/ci.yml): a commit pushed to main whose HEAD carries `pre-release: true` cuts a pre-release, `release: true` cuts a stable release, no trailer runs nothing extra. TheVault needs proper pre-release support for this.

Build:
1. A release-train workflow on push to main (plus workflow_dispatch with a `kind` input of pre-release or release, for re-runs): read the HEAD trailers with `git log -1 --format='%(trailers:key=release,valueonly)'` and `pre-release`. Compute the version from the last stable `v*` tag (ignore pre-release tags when finding the base): stable = next patch (a `version: X.Y.Z` trailer sets it explicitly, for minor or major bumps); pre-release = <next patch or the version trailer>-alpha.N where N is one more than the highest existing alpha tag for that base. Fail if the tag already exists.
2. GitHub-token pushes do not trigger other workflows, so do not rely on a tag push: turn release.yml, android-release.yml and image.yml into reusable workflows (`workflow_call` with `version` and `prerelease` inputs), keep their tag-push triggers for manual tags if that stays simple, and have the train call them in order (desktop and Android in parallel with Android's smoke gate kept, then the image). Write the version into Cargo.toml [workspace.package], both tauri.conf.json files and the Cargo.lock files inside CI before building; commit nothing to main (wharf's approach); create the tag on the built commit and the GitHub Release (marked pre-release for alphas).
3. Pre-release handling everywhere:
   - Image: a pre-release pushes only `ghcr.io/wiltaylor/thevault:<version>` and moves an `edge` tag, never `latest` or `<major>.<minor>`. A stable release pushes `<version>`, `<major>.<minor>`, `latest`, and moves `edge`.
   - Linux packages: map the version per format (deb `0.1.13~alpha.1`; rpm `0.1.13~alpha.1` or Release `0.alpha.1`; AUR pkgver `0.1.13alpha1`) or skip AUR for pre-releases; each must build and sort before the stable version in its package manager.
   - Android versionCode: set explicitly from the version so a pre-release sorts after the previous release and before its stable release, and every new code exceeds the codes already shipped (check the shipped 0.1.12 APK's versionCode with aapt or apkanalyzer). For example ((major*1000+minor)*1000+patch)*100 + (N for alpha.N, 99 for stable), verified against the shipped code.
   - Updaters (crates/sync/src/gate.rs, AppUpdate.kt, desktop self_update.rs) already compare semver with pre-releases. Installed apps must not take a pre-release unless the server they talk to runs a pre-release (a server on 0.1.13-alpha.1 must be able to update its apps to it, since the version gate pauses sync until they match). Add tests.
   - GitHub Release marked pre-release so releases/latest/download links in docs/install.md keep the stable release.
4. Docs: docs/releasing.md (linked from README and docs/android-release.md) describes the trailers, the version rules, the tags, and how to re-run a failed train.

Acceptance criteria:
- actionlint passes on every workflow; shellcheck on new scripts.
- Tests for the version calculation: first pre-release after v0.1.12 is 0.1.13-alpha.1, the next is alpha.2, a stable after them is 0.1.13, a version trailer overrides, an existing tag fails.
- Package version mapping: deb, rpm and (if kept) AUR builds succeed locally for a pre-release version and sort before the stable version (dpkg --compare-versions, rpm comparison, vercmp).
- Android versionCode for 0.1.13-alpha.1 and 0.1.13 computed and compared against the shipped 0.1.12 APK's code.
- Updater tests for the pre-release rule.
- ./dev just check and ./dev just android-test pass.
- A real run is not part of this ticket: record the exact commit to push (an empty commit with the trailer) for the user's approval.
