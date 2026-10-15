---
id: '54'
title: 'Server: SSE event stream and web live updates'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
---

# Server: SSE event stream and web live updates

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Server: push and live events" for open clients (D4): `GET /api/events` SSE stream with events `notification.created|read|dismissed|cleared`, `note.changed`, `device.command`; heartbeat comments; auth like other endpoints. Web client subscribes (EventSource with credentials) and updates the notification badge/panel and open notes' change awareness immediately, falling back to the existing 30 s poll when the stream fails.

Acceptance criteria:
- S1: a client connected to /api/events receives each notification event and note.changed after a write, in order; unauthenticated gets 401.
- S2: a notification created through the API appears in the bell without waiting for the poll (poll interval set long in the test); with the stream blocked, polling still works.
- `./dev just check` and `./dev just test-e2e` pass.
Decisions: D4, D9.

## Worklog

### 2026-10-15T06:06:05Z — /work

Claimed.

### 2026-10-15T06:57:57Z — wil

Verified set to verified.

### 2026-10-15T06:57:57Z — wil

Status set to done.

### 2026-10-15T06:57:57Z — wil

Committed 69f2e04 on ticket/thevault-clients-54. Event bus is state::Events (tokio broadcast, cap 1024); notifications publish under the state.db lock, writer publishes note.changed after commit (Writer::announce). Payloads carry ids only. device.command is a defined LiveEvent variant that nothing publishes yet: the device-commands ticket should call state.events().publish(LiveEvent::DeviceCommand{..}). Web: live.ts connectLive; note.changed only rechecks the open NoteView for that id (bumping the global changes signal re-rendered every list on each remote write and broke popups); query views keep their own refresh. Two conflict-view e2e tests (shell, inline-editing) now block /api/events, since a live page takes the other write before it can go stale. archive.spec.ts:47 raced the optimistic row removal under load; now polls. Server tests: crates/server/tests/events.rs; completeness test skips reading the endless /api/events body.

### 2026-10-15T07:12:49Z — wil

Rebased onto 403c2c4 (ticket 53 devices; additive conflicts in Cargo.lock, api Cargo.toml, state lib.rs) and fast-forwarded main to c2bab8d. just check green; test-e2e green on the rerun (first run had one unrelated failure, theme-settings.spec.ts:184 asset-count, which passed alone and on the rerun).
