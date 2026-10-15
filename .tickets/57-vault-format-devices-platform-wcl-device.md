---
id: '57'
title: 'Vault format: devices/<platform>.wcl, device event catalogue, device commands
  API'
plan: thevault-clients
blocked_by:
- '53'
status: done
verified: verified
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

### 2026-10-15T07:13:23Z — /work

Claimed.

### 2026-10-15T07:32:41Z — wil

Verified set to verified.

### 2026-10-15T07:32:41Z — wil

Status set to done.

### 2026-10-15T07:32:41Z — wil

Done on ticket/thevault-clients-57 (d360e05). Evidence: crates/server/tests/device_commands.rs (S1: device files round-trip byte for byte, unknown events/commands/platforms/missing scripts refused with 400 invalid; task queues declared+enabled send_sms, disabled delete_contact and undeclared show_notification refused, before_create cannot queue, after_create can, results stored, PT25H makes pending expired and reports 409). crates/state/tests/commands.rs and schema devices unit tests. ./dev just check exit 0. Assumptions: command labels limited to the 4 built-ins; script required, function defaults to the label; device named by id or name; TTL fixed 24h; GET commands has no side effects (device POSTs delivered); commands endpoints for own token or admin; API at /api/device-files/{platform}; payloads in docs/scripts.md (handler value {event, device, at, data}, command value {id, command, payload}) for S4 to pin. Not done: WebPush dispatch (ticket 56), change-feed coverage of devices/*.wcl (ticket 55), CLI commands for device files. Merge note: state SCHEMA_VERSION goes to 9 and store LAYOUT_DIRS gains devices; other tickets that bump either will conflict.

### 2026-10-15T07:36:14Z — wil

Merged into main as 51f9586 after rebasing onto 3a99c93. Rebase changes: device_commands moved to state schema v10 (ticket 55 took v9), with a v9->v10 migration test; device file writes now go through Writer::commit so they reach the change feed; auth/route/openapi entries merged with the sync routes. ./dev just check and check.sh pass on the rebased branch.
