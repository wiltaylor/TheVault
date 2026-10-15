---
id: '60'
title: 'crates/sync: offline queue, change-feed pull, three-way merge, conflict copies,
  S3 harness'
plan: thevault-clients
blocked_by:
- '55'
- '59'
status: todo
verified: unknown
---

# crates/sync: offline queue, change-feed pull, three-way merge, conflict copies, S3 harness

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Sync engine" (D19, D20, D27, D28, D29) as crates/sync, used by both apps: durable queue of local writes in the local state.db with base version and base WCL text; push replays queued writes through the home server's write API with If-Match using the device token; on 409 three-way merge (fields individually; top-level content blocks by diff3 over canonical-text hashes); a clash keeps the server version and creates a conflict copy titled "<title> (conflict, <device name>)" linked to the original (choose the link shape and document it - SPEC open question) plus a notification via the API; pull applies `/api/sync/changes` since the cursor to the local vault and reindexes; handshake with /api/sync/info refuses and keeps the queue when the server is newer than supported; records bases with PUT /api/devices/{id}/bases. Assets: a pluggable policy (all assets for desktop; on-demand with size-limited cache and pinned/recent prefetch for Android) - implement both policies. Web: conflict copies display a link to the original (S2).

Acceptance criteria (S3 two-node harness: a crates/embedded device node and a testkit home server in one test):
- Offline edits to different fields and different blocks on both sides merge with no copy.
- An overlapping edit makes exactly one conflict copy and one notification; reordered blocks fall back to a copy.
- A newer server refuses sync and keeps the queue intact.
- The base survives a browser session merge and pruning on the server.
- Deletions and trash moves propagate both ways; definitions are not pushed from the device.
- S2: a conflict copy shows a link to its original.
- `./dev just check` and `./dev just test-e2e` pass.
Decisions: D19, D20, D27, D28, D29.

## Worklog
