---
id: '51'
title: 'Dev container: Tauri Linux and Android toolchains, just recipes'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
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

### 2026-10-15T06:06:02Z — /work

Claimed.

### 2026-10-15T06:40:07Z — wil

Verified set to verified.

### 2026-10-15T06:40:07Z — wil

Status set to done.

### 2026-10-15T06:40:07Z — wil

Image tag is thevault-dev:<first 12 hex of sha256(docker/dev.Dockerfile)> (here 5c69d9c9d2f0), with a thevault.dockerfile-hash label. Tools installed under /usr/local/cargo/bin before CARGO_HOME moves to the /cargo volume, otherwise the volume hides them. 'tauri' is a symlink to cargo-tauri. Android SDK at /opt/android-sdk (world-writable so Gradle can add components), NDK_HOME set for Tauri. GRADLE_USER_HOME=/gradle on volume thevault-gradle. Stub recipes check for apps/desktop/src-tauri, apps/desktop/e2e (desktop-test runs 'xvfb-run -a pnpm -C apps/desktop/e2e test') and apps/android/src-tauri/gen/android/gradlew (android-test runs 'gradlew testDebugUnitTest'); tickets 62/65 may change these commands. just check green. test-e2e: two full runs at load avg ~25 failed 2 and 10 specs, different sets each time; every failing spec passed when rerun alone. The Playwright layer is unchanged, so these are load flakes, not an image regression.
