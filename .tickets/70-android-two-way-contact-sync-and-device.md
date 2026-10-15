---
id: '70'
title: Android two-way contact sync and device commands execution
plan: thevault-clients
blocked_by:
- '66'
- '57'
- '69'
status: done
verified: verified
---

# Android two-way contact sync and device commands execution

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC contact sync and commands on Android (D16, D22, D23, D24): ContactsContract writes (upsert, delete) behind the contacts-write switch; store the vault note id in a custom data row on the contact and the contact id in person.device_contacts; field-level most-recent-wins conflict handling; deletes unlink unless deletion syncing is enabled; device commands: fetch pending on push or periodic check, run declared and enabled commands (send_sms via SmsManager behind the SMS-send switch and SEND_SMS permission, upsert_contact, delete_contact, show_notification), report results.

Acceptance criteria (S5, plus S4 for the script side):
- Fake ContactsContract: a person change upserts the contact with the custom data row; a contact change updates the person; delete unlinks.
- A send_sms command runs only when declared and both switches allow it; results are posted.
- `./dev just android-test` and `./dev just check` pass.
Decisions: D16, D22, D23, D24.

## Worklog

### 2026-10-15T08:47:07Z — /work

Claimed.

### 2026-10-15T09:14:34Z — wil

Verified set to verified.

### 2026-10-15T09:14:34Z — wil

Branch ticket/thevault-clients-70: 3d3e900 is a cherry-pick of ticket 69 (9e7c4d1, unmerged when 70 started), conflicts with 66 resolved as a union; merge 69 before 70. f2cd359 is this ticket. Contacts: ContactsReader writes a custom data row (vnd.android.cursor.item/vnd.dev.wiltaylor.thevault.person; data1 = person id, data2 = JSON of the fields last agreed) and rewrites only kinds that differ; reads expose person and synced. device_contacts.ws settles name/phones/addresses per field against that base (most recent wins when both changed); a contact whose stored person was deleted in the vault is left alone. Person -> contact: DeviceRuntime::queue_person_changes (tracks person versions in device_people, gated on contacts_write) runs after each sync round in Core::sync_now and queues contact_changed for linked persons. Commands: plugin commands.rs + Core::run_commands + JNI runCommands + CommandWorker (device.command push, app open, 15-min fallback beside NotificationWorker). AndroidDeviceData skips divideMessage for bodies <= 70 chars (Robolectric has no ISms service). Not done: 'deletion syncing' opt-in from D16 has no setting anywhere in the spec or server; deletes always unlink, and deleting a phone contact from the vault side is only through the explicit delete_contact command. Tests: ./dev just check; ./dev just android-test (GRADLE_OPTS=-Dorg.gradle.daemon=false avoids a shared-daemon setcwd failure when other worktrees run gradle at once).

### 2026-10-15T09:14:34Z — wil

Status set to done.

### 2026-10-15T09:20:04Z — wil

Merged to main as befb436 (fast-forward). Rebased onto f9a4958, dropping the 69 cherry-pick since 69 landed as f2a8bb6. Conflicts: jni.rs (kept runCommands only), VaultDevicePlugin.kt doc, docs/android-manual-checks.md (71 renumbered it; added non-duplicate checks 44-48 and coverage). Rerun on the rebased branch: just check, plugin fmt+clippy, android-test all pass.
