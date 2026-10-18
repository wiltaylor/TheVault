---
id: '137'
title: 'just desktop-release-test fails from a fresh checkout (so release.yml fails
  in CI): apps/desktop/e2e/appimage.smoke.mjs, through harness.mjs, needs apps/desktop/src-tauri/target/debug/examples/fake-notifyd,
  which only just desktop-test builds'
blocked_by: []
status: done
verified: verified
---

# just desktop-release-test fails from a fresh checkout (so release.yml fails in CI): apps/desktop/e2e/appimage.smoke.mjs, through harness.mjs, needs apps/desktop/src-tauri/target/debug/examples/fake-notifyd, which only just desktop-test builds

Found on ticket 135's dry run: after just desktop-release built and signed the AppImage, the smoke test failed with 'fake-notifyd is missing; run just desktop-test, or set FAKE_NOTIFYD_BIN'. A CI runner starts from a clean checkout, so the release job stops at this step before publishing. Fix is likely desktop-release-test depending on building the fake-notifyd example (cargo build --example fake-notifyd in apps/desktop/src-tauri).

## Worklog

### 2026-10-18T10:41:22Z — /work

Claimed.

### 2026-10-18T10:52:43Z — wil

Verified set to verified.

### 2026-10-18T10:52:43Z — wil

Status set to done.

### 2026-10-18T10:52:43Z — wil

Clean worktree (no apps/desktop/src-tauri/target): ./dev just desktop-release-test with updater.env sourced exited 0; AppImage signed, smoke test ok 1/pass 1/fail 0. Scanned release.yml, android-release.yml, image.yml, docker/release-image.sh: just aur-release reads only desktop-release output (earlier step), just cli-release ran clean from this worktree and exited 0, just android-release needs only web-build plus committed gen/android and the keystore (not run: needs the signing keystore and SDK build time). Nothing else depends on a dev-only build. just check exit 0.
