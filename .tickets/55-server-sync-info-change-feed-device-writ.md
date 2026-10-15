---
id: '55'
title: 'Server: sync info, change feed, device-write versions, base retention'
plan: thevault-clients
blocked_by:
- '53'
status: todo
verified: unknown
---

# Server: sync info, change feed, device-write versions, base retention

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Server: change feed and version handshake" (D20, D27, D29): `GET /api/sync/info` (unauthenticated: protocol, format_version, server_version); `GET /api/sync/changes?since=&limit=` over `versions.seq` covering notes, types, templates, taxonomies, scripts, tasks, devices/*.wcl, deletions and trash moves, with next/latest; writes with a device token always create a new version; `PUT /api/devices/{id}/bases` and pruning never deletes those versions; device tokens cannot write definitions (types, templates, taxonomies, scripts, tasks) through sync paths (they use the normal API while online - document exactly what is enforced).

Acceptance criteria (S1):
- Changes since a seq return every kind of change in order, pages correctly, and include deletions.
- A device-token write right after a browser session write creates two versions; a session write after a session write still merges.
- A recorded base survives pruning; after the device clears it, pruning removes it.
- /api/sync/info works without auth.
- `./dev just check` passes.
Decisions: D20, D27, D29.

## Worklog
