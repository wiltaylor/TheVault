---
id: '78'
title: 'Android app: captured SMS, MMS, RCS, calls and contact changes are not synced
  until the next app open or 15-minute job'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
---

# Android app: captured SMS, MMS, RCS, calls and contact changes are not synced until the next app open or 15-minute job

Found by the emulator run of docs/android-manual-checks.md (checks 29 to 33, 35, 36; Android 15 API 35 emulator, debug APK at 1574931 with the ticket 76 workaround).

Repro: pair the app and finish capture setup, leave the app open in the foreground (or closed), receive an SMS (adb emu sms send 6502530001 hi).

Expected (checks 29 to 33, 35, 36): the message, call or person note reaches the web app within a few seconds.

Actual: the handler runs on the phone and writes the notes locally (notes/message/*.wcl present in the app's vault; their rows sit in sync_queue in .vault/state.db), but nothing reaches the server for at least 60 s. They arrive only when the app is next brought to the foreground or the 15-minute SyncWorker runs.

Cause: a round runs on app open, after a webview write (lib.rs serve -> core.kick) and after a share (Core::share -> kick), and every 15 minutes. The device-capture path (receivers/observers -> DeviceCapture.flush -> RunEventsWorker -> VaultCore.runPending) never calls kick or enqueues a sync, so handler-written notes wait. Contact changes take the same path.

Fix direction: after run_pending writes anything, kick the core (and, since the process may be in the background, enqueue a one-time expedited SyncWorker with a network constraint). Also consider pulling on a note.changed push so server-side changes (checks 37, 38) reach an open app.

Evidence: .scratch/emulator/RESULTS.md checks 29 to 36.

## Worklog

### 2026-10-15T12:11:08Z — wil

Status set to claimed.

### 2026-10-15T12:11:09Z — wil

Fixed in f9b9e17: background captures (RunEventsWorker, shares, device commands) call SyncWorker.soon, which kicks the foreground loop when the app is in front or enqueues one unique expedited network-constrained sync round (2 s in-worker debounce, bursts coalesce). PromptSyncTest and a Rust testkit round-trip prove it; check and android-test pass after rebase onto main. Emulator confirmation pending.

### 2026-10-15T12:11:09Z — wil

Verified set to unmet.

### 2026-10-15T12:11:09Z — wil

Status set to done.

### 2026-10-15T14:00:27Z — wil

Confirmed by the emulator run of docs/android-manual-checks.md on main at 8404c46 (Android 15 x86_64, fresh debug APK, wiped emulator); see .scratch/emulator/RESULTS.md.

### 2026-10-15T14:00:27Z — wil

Verified set to verified.
