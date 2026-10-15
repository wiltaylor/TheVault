---
id: '53'
title: 'Server: devices table, pairing (password and QR code), Settings > Devices,
  CLI'
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
---

# Server: devices table, pairing (password and QR code), Settings > Devices, CLI

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Server: devices and pairing" (D2, D3, D25 settings storage): state.db `devices` table with migration, `POST /api/devices/pair` (password or code), `POST /api/devices/pair-codes`, `GET/PATCH/DELETE /api/devices`, `PUT /api/devices/{id}/push` (store endpoint and WebPush keys; delivery is another ticket). Device tokens have write scope and are bound to the device; revoke deletes token and push endpoint. Settings > Devices page in web/: list, rename, revoke, per-device switches (contacts write, SMS send, enabled commands), exclusion list and skip pattern editors, and a "Pair a device" button showing the QR code (payload `{server_url, code}`) and the code text. CLI `vault device list|rename|revoke|pair-code`. Update OpenAPI list, docs/parity.md, crates/cli/SKILL.md.

Acceptance criteria:
- S1: pair by password and by code; a code works once and expires; the token reaches write endpoints and not admin ones; revoke makes the token fail with 401; settings round-trip.
- S2: Settings > Devices shows a paired device, rename and revoke work, the QR code renders with the code.
- `./dev just check` and `./dev just test-e2e` pass.
Decisions: D2, D3, D25.

## Worklog
