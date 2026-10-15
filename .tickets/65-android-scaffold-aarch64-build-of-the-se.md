---
id: '65'
title: 'Android scaffold: aarch64 build of the server crates, Tauri Android app shell
  builds a debug APK'
plan: thevault-clients
blocked_by:
- '51'
- '59'
status: done
verified: verified
---

# Android scaffold: aarch64 build of the server crates, Tauri Android app shell builds a debug APK

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Resolve SPEC open question 2 first: build crates/embedded (and its server crates: tantivy, bundled rusqlite, pdf-extract) for aarch64-linux-android with cargo-ndk; fix or feature-gate what fails and report it. Then scaffold apps/android: Tauri 2 mobile app bundling web/dist, custom scheme to crates/embedded, minSdk 33, `src-tauri/gen/android` committed, `./dev just android-build` produces a debug APK. Add the empty Kotlin plugin crate plugins/tauri-plugin-vault-device with a JNI round-trip (Kotlin calls a Rust function with no webview) and a Robolectric test harness wired to `just android-test`.

Acceptance criteria:
- `./dev just android-build` produces an installable debug APK (aarch64).
- `./dev just android-test` runs a Robolectric test and a JNI round-trip test green (S5).
- `./dev just check` still passes.
Decisions: D14, D17, D18, D26.

## Worklog

### 2026-10-15T06:43:33Z — /work

Claimed.

### 2026-10-15T07:21:49Z — wil

Verified set to verified.

### 2026-10-15T07:21:49Z — wil

Status set to done.

### 2026-10-15T07:21:49Z — wil

Branch ticket/thevault-clients-65 at 08cde7e, built on ticket 51's branch (5604462), so merge 51 first. SPEC OQ2 answered: crates/embedded (tantivy, bundled rusqlite, pdf-extract) builds and links for aarch64-linux-android unchanged; no feature gates. The app and plugin crates form their own Cargo workspace at apps/android/src-tauri (plugin joins via package.workspace), outside the server workspace and just check. Their lockfile was seeded from the root Cargo.lock. Tauri 2.12 needs compileSdk 36 and AGP 9.3.1 needs build-tools 36, so the dev image moved from 35 to 36. Java in the container sees user.home as '?': ./dev sets ANDROID_USER_HOME, and the plugin's unit tests set user.home to <gradle home>/robolectric-home, so Robolectric's android-all jars are cached in the /gradle volume. Gradle includes (tauri.settings.gradle, tauri.build.gradle.kts) exist only after cargo tauri android build, so android-test depends on android-build. The JNI test loads a host cdylib of the plugin through VAULT_CORE_LIBRARY; without it the test fails (UnsatisfiedLinkError) rather than skipping. The APK was checked with apksigner (v2 verifies) and aapt2 (minSdk 33, arm64-v8a) but never installed on a device; S7 covers that. The debug APK is about 480 MB because of unstripped debug symbols. Package ids: dev.wiltaylor.thevault and dev.wiltaylor.thevault.device.

### 2026-10-15T07:28:20Z — wil

Rebased onto main 05fb185 and fast-forwarded main to 96f3c62. On the rebased branch: just check passed (495 Rust tests, 51 web test files) and just android-test passed. The app lockfile gained qrcode from main's crates and was amended into the commit.
