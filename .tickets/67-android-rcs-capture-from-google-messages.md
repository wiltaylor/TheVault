---
id: '67'
title: Android RCS capture from Google Messages notifications (S5)
plan: thevault-clients
blocked_by:
- '66'
status: done
verified: verified
---

# Android RCS capture from Google Messages notifications (S5)

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC RCS capture (D13): a NotificationListenerService in the Kotlin plugin that parses Google Messages notifications (MessagingStyle extras: conversation title, sender person name, text, timestamp) into rcs_received events with partial = true and a stable device_ref (notification key + message timestamp); ignore SMS already captured from the provider (match on thread/time/text) to avoid duplicates; onboarding step for notification-listener access including Allow restricted settings. Record a few real-format sample notifications as test fixtures.

Acceptance criteria (S5):
- Fixture notifications produce the expected events; group chats map to one conversation; duplicates of provider SMS are dropped; non-Messages notifications are ignored.
- `./dev just android-test` passes.
Decisions: D13.

## Worklog

### 2026-10-15T08:29:50Z — /work

Claimed.

### 2026-10-15T08:55:01Z — wil

Verified set to verified.

### 2026-10-15T08:55:01Z — wil

Status set to done.

### 2026-10-15T08:55:01Z — wil

Done on ticket/thevault-clients-67 (be2cee4), built on 66's rebased branch (abcec58), which was not on main at claim time: merge 66 first and 67 is a fast-forward on top of it. Evidence: ./dev just android-test exit 0 (RcsCaptureTest 11/11, CaptureSetupActivityTest 4/4, every other JVM suite green); ./dev just check exit 0 (550 Rust, 546 web). The first android-test run failed only in plugins/tauri-plugin-vault-device/tests/core.rs a_write_kicks_the_foreground_loop (30 s deadline at load avg 54); it passed alone and on the full rerun. Filed as ticket 74. Caveats: the fixtures under android/src/test/resources/rcs/ are reconstructions of the extras Google Messages posts (android.messages bundles, sender_person, android.messagingUser, shortcut id = conversation id), not dumps from a real phone. S7 is the check against the live app, and a real dump can be dropped into the same JSON shape. Decisions taken: thread_ref = notification shortcutId, or the key when there is none. device_ref = rcs:<notification key>:<message ms>. The SMS/MMS duplicate check matches received provider rows on text (whitespace-collapsed) within +/-120 s of DATE or DATE_SENT, because thread ids and sender identities differ between the notification and the provider. Without READ_SMS nothing is matched and everything is kept. A seen list (last 500 refs, SharedPreferences thevault.device.rcs) stops reposts from re-enqueueing. The listener waits Device.retryDelayMs before reading, so a slow provider write still matches. The onboarding step 'RCS messages' comes after Messages; capture_status gains notificationAccess. The web side does not yet read notificationAccess.

### 2026-10-15T09:04:33Z — wil

Merged: rebased onto ae9dc92 (clean), ./dev just check exit 0 (550 Rust, 546 web) and ./dev just android-test exit 0 (RcsCaptureTest 11/11 alongside 69's NotificationMirrorTest 10/10), then fast-forwarded main to 9f1e45e.
