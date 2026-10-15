---
id: '68'
title: 'Android app: onboarding and pairing (QR scan), keystore, sync loop, share
  target, asset cache'
plan: thevault-clients
blocked_by:
- '65'
- '60'
- '53'
status: todo
verified: unknown
---

# Android app: onboarding and pairing (QR scan), keystore, sync loop, share target, asset cache

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Android app" shell parts (D2, D10 onboarding, D19, D20): onboarding (server URL + password, or scan the pairing QR code), device token in the Android Keystore, crates/sync loop (foreground on app open and after writes; WorkManager periodic sync in background), on-demand asset policy with size-limited cache and prefetch of pinned/recent notes' assets, share target via tauri-plugin-mobile-sharetarget (or fork) that copies shared content:// files natively into local assets and creates fleeting notes (works offline; event share_received through the device runtime). S7 manual checklist file docs/android-manual-checks.md started here.

Acceptance criteria:
- S5: share intents with text, URL, one image and several files become queued fleeting notes with assets.
- S3 already covers sync; add a Robolectric test that the background sync worker runs the sync engine via JNI.
- `./dev just android-build` and `./dev just android-test` pass.
Decisions: D2, D10, D19, D20.

## Worklog
