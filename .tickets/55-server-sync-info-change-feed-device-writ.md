---
id: '55'
title: 'Server: sync info, change feed, device-write versions, base retention'
plan: thevault-clients
blocked_by:
- '53'
status: done
verified: verified
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

### 2026-10-15T06:48:59Z — /work

Claimed.

### 2026-10-15T07:03:20Z — wil

Verified set to verified.

### 2026-10-15T07:03:20Z — wil

Status set to done.

### 2026-10-15T07:03:20Z — wil

Commit c1201ab on ticket/thevault-clients-55, which also merges ticket/thevault-clients-53 (unmerged on main at claim time; one doc-comment conflict in crates/api/src/lib.rs resolved). Feed cursor is a new state.db 'changes' journal (schema 9), not versions.seq: merged browser saves rewrite their versions row in place and definitions/deletions have no versions. devices/ is not in store::LAYOUT_DIRS (ticket 57's job); the feed reads synced files from disk via Writer::read_synced so devices/*.wcl already works. Pruning has no server trigger yet; S1 prunes by opening state.db beside the running server. Bases protect exactly the recorded version (not later ones). Notes copied in by hand and never adopted into state.db are not in the feed until first written. Schema bump 8->9 will conflict with any other ticket bumping it.

### 2026-10-15T07:23:23Z — wil

Merged to main as 05fb185 (rebased onto 3b1f4d4; one conflict in Writer::put_type resolved to self.commit(changes)); ./dev just check passed on the rebased branch.
