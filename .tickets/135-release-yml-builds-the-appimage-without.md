---
id: '135'
title: release.yml builds the AppImage without TAURI_SIGNING_PRIVATE_KEY, so a CI
  release (and the image built from it) carries no updater signature and installed
  desktop apps will not update to it; pass the updater key as a repository secret
  and attach the .AppImage.sig
blocked_by: []
status: done
verified: verified
---

# release.yml builds the AppImage without TAURI_SIGNING_PRIVATE_KEY, so a CI release (and the image built from it) carries no updater signature and installed desktop apps will not update to it; pass the updater key as a repository secret and attach the .AppImage.sig

## Worklog

### 2026-10-18T10:33:02Z — /work

Claimed.

### 2026-10-18T10:40:30Z — wil

Verified set to verified.

### 2026-10-18T10:40:30Z — wil

Status set to done.

### 2026-10-18T10:40:30Z — wil

Commit 4bc28e0 on ticket/135. actionlint clean. Dry run: updater.env key text (plus a trailing newline) through the workflow's exact build command signed TheVault_0.1.11_amd64.AppImage; the .sig verifies (minisign, pynacl) against tauri.conf.json's pubkey, key id B978222835AE6122, and a truncated file fails. docker/release-image.sh (IMAGE=thevault-t135-dryrun, no push, dummy APK/CLI) served the AppImage with /api/clients signature equal to the .sig. ./dev just check rc 0. The smoke test in desktop-release-test failed on a clean worktree for want of fake-notifyd: filed 137, which will stop the CI release job before it publishes.
