---
id: '57'
title: 'Vault format: devices/<platform>.wcl, device event catalogue, device commands
  API'
plan: thevault-clients
blocked_by:
- '53'
status: todo
verified: unknown
---

# Vault format: devices/<platform>.wcl, device event catalogue, device commands API

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Vault format: devices and phone types" (file kind part) and "Server: device commands" (D5, D22): new vault file kind `devices/<platform>.wcl` with `handler "<event>" { script, function }` and `command "<name>" { script, function }`, validated against the closed event list (sms_received, sms_sent, mms_received, rcs_received, call_ended, contact_changed, contact_deleted, share_received) and built-in commands (send_sms, upsert_contact, delete_contact, show_notification); documented in docs/vault-format.md with a structural schema; API to read/write these files like types. `device_commands` table, `vault::device_command(device, name, payload)` in after_* handlers and tasks (rejected elsewhere and when not declared or not enabled for the device), `GET /api/devices/{id}/commands?status=pending`, `POST .../commands/{cid}/result`, expiry. Write each event payload and command payload shape into docs/scripts.md (SPEC open question 1).

Acceptance criteria (S1):
- Invalid event or command names are schema errors; a valid file round-trips byte-for-byte.
- A task queues a declared and enabled command; an undeclared or disabled one is refused; a before_* handler cannot queue one; expired commands report expired; results are stored.
- `./dev just check` passes.
Decisions: D5, D22.

## Worklog
