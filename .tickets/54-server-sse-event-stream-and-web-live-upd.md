---
id: '54'
title: 'Server: SSE event stream and web live updates'
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
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
