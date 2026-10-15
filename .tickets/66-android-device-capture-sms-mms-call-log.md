---
id: '66'
title: 'Android device capture: SMS, MMS, call log, contacts readers, receivers, WorkManager
  catch-up, first-run import (S5)'
plan: thevault-clients
blocked_by:
- '65'
- '61'
status: todo
verified: unknown
---

# Android device capture: SMS, MMS, call log, contacts readers, receivers, WorkManager catch-up, first-run import (S5)

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Android app" capture parts (D7, D17, D24): in plugins/tauri-plugin-vault-device: provider readers (Telephony SMS/MMS incl. MMS parts as attachments, CallLog, ContactsContract), SMS received receiver, MMS by observing the Telephony provider, phone-state receiver triggering a call log re-read on call end, contacts ContentObserver plus a WorkManager content-URI job, periodic WorkManager catch-up since stored cursors, first-run import screen (off by default, date range); all paths normalise numbers to E.164 with libphonenumber and enqueue events through JNI into the device event queue (no webview needed); implement DeviceData for the device module; runtime permission requests in onboarding including the Allow restricted settings guidance for SMS (Android 15).

Acceptance criteria (S5 Robolectric):
- Readers return the right rows from fake providers; receivers enqueue sms_received, mms_received, call_ended, contact_changed with normalised numbers; catch-up skips rows already captured (device_ref).
- `./dev just android-test` and `./dev just check` pass.
Decisions: D7, D17, D23, D24.

## Worklog
