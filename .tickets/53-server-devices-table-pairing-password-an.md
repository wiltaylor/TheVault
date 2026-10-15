---
id: '53'
title: 'Server: devices table, pairing (password and QR code), Settings > Devices,
  CLI'
plan: thevault-clients
blocked_by: []
status: done
verified: unmet
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

### 2026-10-15T06:06:02Z — /work

Claimed.

### 2026-10-15T06:48:30Z — wil

Verified set to unmet.

### 2026-10-15T06:48:30Z — wil

Status set to done.

### 2026-10-15T06:48:30Z — wil

Branch ticket/thevault-clients-53, commit b2fa3e9. Verdict unmet only for '`./dev just test-e2e` passes': two full runs at load avg 11-20 (other fleet workers) failed 11 and then 3 different specs, none touching devices (date-views, definitions, deletes, editor, fleeting, history, theme-settings timeouts; archive.spec.ts:123 reads the note before the unarchive write lands; graph). Every failed spec passed on --last-failed, so all 251 tests passed, but never in one run. See #72. S1 (crates/server/tests/devices.rs, 5 tests), S2 (web/e2e/devices.spec.ts, 2 tests) and ./dev just check all pass. Assumptions: DELETE /api/devices/{id} deletes the device row too (FK cascade from tokens), not just the token; pair codes live in state.db (hashed, 10 min default, ttl_secs 1..3600 so S1 can test expiry); QR SVG rendered server-side with the qrcode crate (new dependency) and returned as qr_svg; added 'vault device settings <id>' (JSON on stdin) for UI/CLI parity (D32) beyond the four listed subcommands; GET /api/devices shows push as a bool, never the endpoint or keys. Token-to-session exchange from note 04 is not in SPEC's devices section and was not built. state.db schema is now version 8; any other ticket that bumps it will conflict at merge.

### 2026-10-15T06:59:22Z — wil

Merged to main as 403c2c4 (fast-forward after rebase onto 5604462). Rebase conflict in crates/api/src/lib.rs module docs only; also made auth::as_owner (the new in-process owner router) insert GrantedScope(Admin) so PUT /api/devices/{id}/push works there. Rebased branch: ./dev just check exit 0; test-e2e 246 passed, 3 failed (archive.spec.ts:47 race, date-views kanban timeout, pins drag), all three pass on rerun; devices specs passed in the full run. test-e2e still not one clean run, so verified stays unmet (#72).
