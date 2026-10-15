---
id: '66'
title: 'Android device capture: SMS, MMS, call log, contacts readers, receivers, WorkManager
  catch-up, first-run import (S5)'
plan: thevault-clients
blocked_by:
- '65'
- '61'
status: done
verified: verified
---

# Android device capture: SMS, MMS, call log, contacts readers, receivers, WorkManager catch-up, first-run import (S5)

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Android app" capture parts (D7, D17, D24): in plugins/tauri-plugin-vault-device: provider readers (Telephony SMS/MMS incl. MMS parts as attachments, CallLog, ContactsContract), SMS received receiver, MMS by observing the Telephony provider, phone-state receiver triggering a call log re-read on call end, contacts ContentObserver plus a WorkManager content-URI job, periodic WorkManager catch-up since stored cursors, first-run import screen (off by default, date range); all paths normalise numbers to E.164 with libphonenumber and enqueue events through JNI into the device event queue (no webview needed); implement DeviceData for the device module; runtime permission requests in onboarding including the Allow restricted settings guidance for SMS (Android 15).

Acceptance criteria (S5 Robolectric):
- Readers return the right rows from fake providers; receivers enqueue sms_received, mms_received, call_ended, contact_changed with normalised numbers; catch-up skips rows already captured (device_ref).
- `./dev just android-test` and `./dev just check` pass.
Decisions: D7, D17, D23, D24.

## Worklog

### 2026-10-15T07:56:43Z — /work

Claimed.

### 2026-10-15T08:29:07Z — wil

Verified set to verified.

### 2026-10-15T08:29:07Z — wil

Status set to done.

### 2026-10-15T08:29:07Z — wil

Done on ticket/thevault-clients-66 (3843936). The branch merges ticket/thevault-clients-61 (3fae4f6, carrying 57) because DeviceData and the device queue live there; 5 additive conflicts with main (embedded lib.rs, writer devices.rs/lib.rs, docs/scripts.md, docs/vault-format.md) were resolved by keeping both sides. Merge 57 and 61 first; whoever merges 61 meets the same conflicts. Evidence: ./dev just android-test exit 0 (aarch64 APK built; 27 JVM tests, 0 failures: ReadersTest 7, CaptureTest 9, CaptureSetupActivityTest 3, VaultCoreTest 7 JNI, VaultDevicePluginTest 1). ./dev just check exit 0 (550 Rust tests, 546 web tests). Left for others: web onboarding does not call open_capture_setup or set_device_id yet (ticket 68 owns onboarding). Device settings from the server (exclusions, switches) are not pushed into the runtime here; that is the sync loop's job. Contacts upsert/delete and sms_send are implemented but have no S5 test. The APK was never installed on a phone; S7 covers that. Assumptions: the device id lives in plugin SharedPreferences, and handlers wait until it is set. Catch-up runs hourly. Capture starts at the newest rows. The import covers SMS, MMS and calls in the date range, plus contacts changed within it. MMS also triggers the content-URI job. sms_send returns device_ref null.

### 2026-10-15T08:37:55Z — wil

Merged to main as abcec58 (fast-forward from f1579e8). Rebased onto main: 57 and 61 had landed (51f9586, 39d9091), so the merge of 61 dropped out. Ticket 68 had landed its own process core and JNI API, so I integrated rather than duplicated. The device runtime now lives in 68's Core (Core::device and runtime) and opens through a new JNI VaultCore.openDevice(deviceId, bridge) after VaultCore.open. The device id comes from PairingStore.deviceId (a small getter I added), so the set_device_id command and the SharedPreferences id were removed. The plugin opens CaptureSetupActivity once after pairing until setup is done. On the rebased branch: ./dev just android-test exit 0 (APK built, 43 JVM tests in 9 classes, 0 failures, app workspace cargo tests ok) and ./dev just check exit 0 (550 Rust, 546 web). No Playwright specs added or changed.
