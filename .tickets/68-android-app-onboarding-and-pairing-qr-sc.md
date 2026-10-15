---
id: '68'
title: 'Android app: onboarding and pairing (QR scan), keystore, sync loop, share
  target, asset cache'
plan: thevault-clients
blocked_by:
- '65'
- '60'
- '53'
status: done
verified: verified
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

### 2026-10-15T07:50:04Z — /work

Claimed.

### 2026-10-15T08:20:18Z — wil

Verified set to verified.

### 2026-10-15T08:20:18Z — wil

Status set to done.

### 2026-10-15T08:20:18Z — wil

Branch ticket/thevault-clients-68: f8c6f7f is a cherry-pick of ticket 60's d0df92e (crates/sync was not on main; 60's branch sits on pre-rebase 55 and a full merge conflicts in 9 files), then 76c0744 is this ticket. Merge 60 first, or drop f8c6f7f if 60 lands rebased.

Design: plugins/tauri-plugin-vault-device owns the Rust core (src/core.rs): one Embedded per process at <Context.dataDir>/vault (moved from app_data_dir/vault so Kotlin can find it), shared by the Tauri scheme handler and JNI. Pairing in src/pair.rs; share in src/share.rs builds the share_received payload and makes the fleeting note directly (ticket 61's runtime is not merged; follow-up ticket 73 routes it through device_share.ws). Share target is our own ShareActivity, not tauri-plugin-mobile-sharetarget: that plugin delivers to JS and needs a webview. Token: AES-GCM key in AndroidKeyStore (PairingStore/KeystoreCipher); tests swap in a fake cipher because Robolectric has no AndroidKeyStore. Permission steps of onboarding are left to ticket 66.

Evidence: ./dev just android-test exit 0 (APK built; Rust 7 unit + 5 core-vs-vault-server + 1 app; Kotlin 19 tests in 6 classes, 0 skipped); ./dev just check exit 0 (537 Rust, 546 web). Mutation: SyncWorker not calling VaultCore.sync turned 3 of 4 SyncWorkerTest red. gradle forkEvery=1 because the JNI library loads into one class loader per JVM. Never installed on a phone; docs/android-manual-checks.md is the S7 list.

### 2026-10-15T08:29:07Z — wil

Merged: rebased onto main 1e79c5b (dropped the f8c6f7f cherry-pick, identical to 261f4da), amended the app lockfile for embedded's new deps, fast-forwarded main to f1579e8. On the rebased branch: just check exit 0 (550 Rust, 546 web), just android-test exit 0, Kotlin 19 tests rerun with --rerun, 0 failed.
