---
id: '51'
title: 'Dev container: Tauri Linux and Android toolchains, just recipes'
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
---

# Dev container: Tauri Linux and Android toolchains, just recipes

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Extend docker/dev.Dockerfile and the ./dev wrapper so every later ticket builds and tests the apps in the container.

Scope:
- Linux Tauri deps: webkit2gtk-4.1, libayatana-appindicator, librsvg, libsecret, xvfb, webkit2gtk-driver; tauri-cli 2 and tauri-driver installed.
- Android: JDK 17, Android SDK (platform 35, build-tools), NDK, cargo-ndk, Rust targets aarch64-linux-android (and x86_64-linux-android for tests), Gradle cache volume.
- ./dev rebuilds the image when docker/dev.Dockerfile changes (hash label), not only when missing.
- just recipes: `desktop-build`, `desktop-test` (S6), `android-build` (debug APK), `android-test` (Robolectric, S5). Recipes may be stubs that fail clearly until their tickets land, but must exist and be documented in the justfile.
- Keep `./dev just check` and `./dev just test-e2e` unchanged and green.

Acceptance criteria:
- `./dev tauri --version`, `./dev tauri-driver --help`, `./dev sdkmanager --list_installed`, `./dev cargo ndk --version` all work.
- Editing the Dockerfile triggers a rebuild on the next ./dev call.
- `./dev just check` passes.

## Worklog
