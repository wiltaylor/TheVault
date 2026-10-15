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
status: todo
verified: unknown
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
