---
id: '112'
title: Apps update themselves before syncing when the server is newer, and the Android
  updater resumes and verifies downloads
blocked_by: []
status: done
verified: verified
---

# Apps update themselves before syncing when the server is newer, and the Android updater resumes and verifies downloads

The user decided (2026-10-18): a client updates itself when it detects the server runs a newer version, so the server can be upgraded first and every app catches up before it reads anything new. This replaces the apps-first release order in docs/android-release.md "Releasing a format change" (ticket 100).

Found on the live phone (0.1.7, Samsung SM-F976B): the in-app update to 0.1.8 failed with "the download did not match the server's SHA-256" although the server serves the exact bytes (sha256 and size checked through /api/clients/TheVault_0.1.8_aarch64.apk). AppUpdateWorker.download (plugins/tauri-plugin-vault-device/android/src/main/java/dev/wiltaylor/thevault/device/AppUpdateWorker.kt) streams 68 MB with a 60 s read timeout, never compares the byte count with the manifest's `size` or Content-Length, cannot resume, and reports a mismatch without retrying. Likely cause: a truncated download over Tailscale on mobile data (not confirmed; the phone's status message holds the hash it got).

Build:
1. Version gate, Android: on every sync round (and app open), compare GET /api/version `server_version` with the installed app version (semver, as AppUpdate.compare). When the server is newer and /api/clients offers a release (or, for a debug app, debug) APK for this ABI with version >= the server's, the app (a) stops pulling vault files and type/script/device-file changes from the server (pushing local changes may continue only if that is safe with the server's newer version; if unsure, pause both), (b) runs the update now (AppUpdateWorker), (c) shows "Updating TheVault to X before syncing" in Settings > Devices > This device and a notification, and (d) resumes sync after the new version starts. When no such build is published, sync continues as today and This device shows a warning that the server is newer than the app and no matching app build is published. A failed update keeps sync paused, retries with backoff, and after 3 failures resumes sync with the warning.
2. Version gate, desktop: the same rule for the Linux desktop app (apps/desktop/src-tauri/src/self_update.rs uses the Tauri updater against the server): server newer and a signed build published -> pause pulling, update, restart, resume; none published -> warn and keep syncing.
3. Android updater robustness: compare the downloaded byte count with the manifest `size` and Content-Length; resume a short download with an HTTP Range request (the server's /api/clients handler must answer Range with 206; add it if missing, with a test); retry up to 3 times before reporting a mismatch; on a mismatch show the hash and byte count received in the notification and This device; request a silent update (PackageInstaller SessionParams.setRequireUserAction(USER_ACTION_NOT_REQUIRED) on Android 12+) and fall back to the user prompt when Android requires it.
4. Docs: docs/android-release.md and docs/clients.md describe the new order (server first, apps follow automatically) and keep the apps-first order as the fallback when no build is published.

Acceptance criteria:
- Robolectric tests: server newer with a matching build pauses pulling and enqueues the update; server newer with no build keeps syncing and records the warning; same version syncs normally; 3 failed updates resume sync with the warning.
- Robolectric or unit test: a truncated download is resumed with Range and then verifies; a download that stays wrong after 3 tries reports the received hash and size.
- Server test: GET /api/clients/<file> with a Range header answers 206 with the right bytes.
- Desktop: a unit test of the gate decision (server newer + build, newer + no build, same).
- ./dev just check and ./dev just android-test pass.

## Worklog

### 2026-10-18T03:30:49Z — /work

Claimed.

### 2026-10-18T04:04:41Z — wil

Verified set to verified.

### 2026-10-18T04:04:41Z — wil

Status set to done.

### 2026-10-18T04:04:41Z — wil

Branch ticket/112, commit 488119b. The pause lives in crates/sync/src/gate.rs (decide, apk_for, UpdateGate) and Engine::sync_gated: the Android core (plugins/.../src/core.rs sync_now) and the desktop sync loop (apps/desktop/src-tauri/src/sync_loop.rs Gate::check) both call it before any push or pull; it pauses both directions. Android: Kotlin passes {version, build, arches, failed} through VaultCore.setApp (UpdateGate.kt keeps failures in the thevault-update prefs); SyncWorker enqueues the gate update (unique 'thevault-update-gate', KEEP, initial delay 0/1/4/16 min). Desktop: GateUpdater runs Updates::prepare (same Tauri updater download) then install_ready + restart. The server's /api/clients Range support already existed; the test now covers 'bytes=N-' with If-Range. Silent install (USER_ACTION_NOT_REQUIRED, fallback to the confirm notice) already existed and stays covered by AppUpdateTest. Not exercised: a real phone or a real AppImage restart (no device); just desktop-test (tauri-driver E2E) was not run, only the desktop crate's cargo test --lib and clippy. A wrong-length complete download now counts as wrong bytes and starts over; only a dropped connection (IOException, or fewer bytes than Content-Length) is resumed.
