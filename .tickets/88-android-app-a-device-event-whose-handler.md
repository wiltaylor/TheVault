---
id: '88'
title: 'Android app: a device event whose handler fails for good is never shown anywhere'
blocked_by: []
status: done
verified: verified
---

# Android app: a device event whose handler fails for good is never shown anywhere

A device event whose handler fails three passes is marked `failed` and left alone (crates/embedded/src/device.rs:52,456). Nothing shows it: `RunEventsWorker` discards the result (plugins/tauri-plugin-vault-device/android/src/main/java/dev/wiltaylor/thevault/device/CaptureWork.kt:111), `VaultCore.events()` (plugins/tauri-plugin-vault-device/src/jni.rs, around line 368) is never called from Kotlin or the web app, and the Rust side logs with `eprintln!`, which goes nowhere on Android (core.rs:303). A user whose SMS never became a note has no way to find out why.

Build:
1. In the app, Settings > Devices > This device gets a "Capture problems" section listing failed and skipped device events, newest first: event name, when, the sender or contact shown in the event, the handler error message, and the attempts. Each failed event has Retry (re-queue for another three passes) and Dismiss. Show the count of failed events as a badge on This device.
2. When an event is marked failed for good, the phone shows one notification ("A message could not be saved"), grouped so a burst of failures is one notification, which opens This device.
3. The phone reports each finally-failed event to the home server, which adds one notification under the bell (source `device:<device id>`), so the web app shows it too. Use the existing device token and an existing or new authenticated route; document the route in docs/scripts.md.
4. Android logs from Rust go to logcat (android_logger or equivalent) instead of `eprintln!`.

Acceptance criteria:
- A device test makes a handler fail three times and asserts the event is listed with its message, Retry re-runs it, and Dismiss hides it.
- The server test asserts a reported failure appears as one bell notification with source `device:<id>`.
- Robolectric or instrumented test for the phone notification grouping.
- `./dev just check` and the Android unit tests pass.

## Worklog

### 2026-10-17T20:46:33Z — /work

Claimed.

### 2026-10-17T21:07:19Z — wil

Verified set to verified.

### 2026-10-17T21:07:19Z — wil

Status set to done.

### 2026-10-17T21:07:19Z — wil

Done on ticket/88 at 5d0e6d9. Embedded server routes GET /api/app/device-events, POST /api/app/device-events/{id}/retry|dismiss (Core hands the runtime over with Embedded::set_device). device_events gained notified/reported columns (migrated; existing failed rows count as already announced) and a dismissed status, swept after 7 days like done/skipped. Phone notification: CaptureProblems.kt, one id + group, count accumulates while showing. Server: POST /api/devices/{id}/event-failures (write scope, own device or admin), documented in docs/scripts.md; the core reports at the end of each sync round, 4xx counts as reported. NotificationMirror skips source device:<own id>. Logging: plugin uses log + android_logger and forwards stderr to logcat (src/logging.rs) so eprintln! in embedded/writer/sync lands too; compiled for aarch64 but not observed on a phone. UI in web/src/pages/CaptureProblems.tsx, mounted at the end of ThisDevice, so ticket 91 can sit beside it. FakeHome.notify gained an optional source parameter.
