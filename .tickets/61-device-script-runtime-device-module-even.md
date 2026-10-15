---
id: '61'
title: 'Device script runtime: device module, event queue, exclusions, default device
  scripts (S4)'
plan: thevault-clients
blocked_by:
- '52'
- '57'
- '58'
- '59'
status: done
verified: verified
---

# Device script runtime: device module, event queue, exclusions, default device scripts (S4)

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Script engine on the device" (D5, D6, D23, D24, D25): in crates/embedded (or a crate it uses), a durable device event queue in the local state.db; a runner that loads devices/android.wcl and scripts/*.ws from the local vault and runs the matching handler with Kind::Device; a `device` module (sms.list/get, calls.list, contacts.list/get, contacts.upsert/delete behind the contacts-write switch, sms.send behind the SMS-send switch) implemented over a `DeviceData` trait that the Android app implements later and tests fake; exclusion list and skip pattern applied before handlers; default scripts scripts/device_messages.ws, device_calls.ws, device_contacts.ws, device_share.ws and a default devices/android.wcl shipped as built-in defaults and in the starter vault. Default scripts: messages and calls become message/call/conversation notes linked to person by E.164 number (payload carries normalised numbers) or RCS display name; unknown numbers create no person; de-dupe on device_ref; contact events sync with person (two-way fields, unlink on delete) per D16; share_received makes a fleeting note.

Acceptance criteria (S4: fake DeviceData and canned events):
- SMS, MMS, RCS (partial) and calls produce the right notes and person links; unknown numbers create no person; repeated events do not duplicate.
- Exclusion list and skip pattern drop events; switches block contacts writes and sms.send.
- A command queued on the server and fetched runs only if declared and enabled.
- `./dev just check` passes.
Decisions: D5, D6, D15, D16, D23, D24, D25.

## Worklog

### 2026-10-15T07:33:25Z — /work

Claimed.

### 2026-10-15T07:56:26Z — wil

Verified set to verified.

### 2026-10-15T07:56:26Z — wil

Status set to done.

### 2026-10-15T07:56:26Z — wil

Done on ticket/thevault-clients-61 (3f726bd), which sits on top of ticket 57's unmerged commit afa89ef (fast-forwarded, because the device file kind lives there); if 57 is rebased before merging, rebase this branch too. Evidence: crates/embedded/tests/device.rs (S4: SMS/MMS/RCS/calls -> message/call/conversation with person links, unknown numbers create no person, repeats de-duplicate on device_ref, exclusions and skip pattern -> skipped, contact sync both ways with unlink on delete, switches block contacts_upsert/delete and sms_send, queue survives restart, retries); crates/server/tests/device_runtime.rs (task on a testkit server queues send_sms and show_notification, device fetches with its token and runs only declared+enabled commands, results stored; default scripts report no problems on the server); ./dev just check exit 0. Assumptions: device module functions are device::sms_list etc. (no dots in wscript names) plus device::id and device::show_notification; queue tables device_events/device_runtime created by embedded in state.db outside state migrations to avoid a SCHEMA_VERSION clash with ticket 60; device_contacts keys are device_<id> because ULIDs are not identifiers (ticket 58's vocabulary could not hold raw ids); skip pattern applies to SMS only; contact most-recent-wins compares contact.updated with the person's updated; new phone contacts create a person; delete-sync opt-in not implemented; show_notification lives in device_messages.ws; built-in defaults are used, not written, on the device. Not done: Kotlin DeviceData implementation and JNI wiring (Android tickets), fetching commands over HTTP is left to the app/sync loop.

### 2026-10-15T08:01:03Z — wil

Merged into main as 39d9091 (rebased onto 261f4da after 57 landed as 51f9586; embedded/src/lib.rs conflict with ticket 60 resolved by keeping both). ./dev just check exit 0 on the rebased branch. No Playwright specs added or changed.
