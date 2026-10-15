---
id: '65'
title: 'Android scaffold: aarch64 build of the server crates, Tauri Android app shell
  builds a debug APK'
plan: thevault-clients
blocked_by:
- '51'
- '59'
status: todo
verified: unknown
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
