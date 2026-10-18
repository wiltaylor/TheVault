---
id: '137'
title: 'just desktop-release-test fails from a fresh checkout (so release.yml fails
  in CI): apps/desktop/e2e/appimage.smoke.mjs, through harness.mjs, needs apps/desktop/src-tauri/target/debug/examples/fake-notifyd,
  which only just desktop-test builds'
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# just desktop-release-test fails from a fresh checkout (so release.yml fails in CI): apps/desktop/e2e/appimage.smoke.mjs, through harness.mjs, needs apps/desktop/src-tauri/target/debug/examples/fake-notifyd, which only just desktop-test builds

Found on ticket 135's dry run: after just desktop-release built and signed the AppImage, the smoke test failed with 'fake-notifyd is missing; run just desktop-test, or set FAKE_NOTIFYD_BIN'. A CI runner starts from a clean checkout, so the release job stops at this step before publishing. Fix is likely desktop-release-test depending on building the fake-notifyd example (cargo build --example fake-notifyd in apps/desktop/src-tauri).

## Worklog
