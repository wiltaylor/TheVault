---
id: '76'
title: 'Android app aborts on every launch: std File::lock is unsupported on Android,
  so opening the local vault fails'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
---

# Android app aborts on every launch: std File::lock is unsupported on Android, so opening the local vault fails

Found by the emulator run of docs/android-manual-checks.md (Android 15, API 35, x86_64 emulator, debug APK from `just android-build x86_64` at 1574931). It blocks every check: the app, the share target, the receivers and the workers all open the local vault first.

Repro: install the debug APK on any Android device or emulator and open it.

Expected: the onboarding screen "Connect TheVault to your server" (check 1).

Actual: the process aborts before any window shows. logcat:

    RustStdoutStderr: thread '<unnamed>' panicked at src/lib.rs:47:10:
    RustStdoutStderr: TheVault failed to start: PluginInitialization("vault-device", "opening vault /data/user/0/dev.wiltaylor.thevault.debug/vault")
    libc: Fatal signal 6 (SIGABRT) ... (.thevault.debug)

Root cause: store::Vault::open -> commit::recover -> Vault::lock_journal calls `std::fs::File::lock()` (crates/store/src/commit.rs:196). Rust's std implements File::lock only for target_os linux, the BSDs, illumos, aix, cygwin and Apple; for target_os = "android" it returns `Unsupported: lock() not supported`. A probe binary built with cargo ndk for x86_64-linux-android (Rust 1.97.1) prints `lock: Err(Error { kind: Unsupported, message: "lock() not supported" })` on the emulator. The aarch64 build has the same target_os, so a phone fails the same way. Host-side tests never see it: the plugin's Rust tests and the Robolectric tests load a Linux host build.

Fix direction: on Android, take the lock with flock(2) through libc (or a crate such as fs4/fd-lock) instead of std's File::lock, and add a test that runs on an Android target or at least compiles the lock path for target_os = "android".

The plugin also panics (aborts) instead of showing an error when the core cannot open; worth a readable failure screen.

Evidence: .scratch/emulator/logs/launch-crash-excerpt.txt, .scratch/emulator/logs/launch-crash-logcat.txt, .scratch/emulator/logs/file-lock-probe.txt, .scratch/emulator/shots/launch-crash-home.png (the launcher, where the app falls back to).

## Worklog

### 2026-10-15T11:31:03Z — wil

Status set to claimed.

### 2026-10-15T11:31:03Z — wil

Fixed in 229d2f6: the journal lock uses rustix flock on every target; clippy.toml bans std File::lock and friends; scripts/android-check.sh lints store and embedded for x86_64-linux-android; the plugin no longer aborts when the core fails to open, the app shows a 'TheVault could not start' page. ./dev just check passes. Emulator launch confirmation pending the re-run.

### 2026-10-15T11:31:03Z — wil

Verified set to unmet.

### 2026-10-15T11:31:04Z — wil

Status set to done.

### 2026-10-15T14:00:27Z — wil

Confirmed by the emulator run of docs/android-manual-checks.md on main at 8404c46 (Android 15 x86_64, fresh debug APK, wiped emulator); see .scratch/emulator/RESULTS.md.

### 2026-10-15T14:00:27Z — wil

Verified set to verified.
