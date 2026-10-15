---
id: '70'
title: Android two-way contact sync and device commands execution
plan: thevault-clients
blocked_by:
- '66'
- '57'
- '69'
status: todo
verified: unknown
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
