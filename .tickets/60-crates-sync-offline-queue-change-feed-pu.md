---
id: '60'
title: 'crates/sync: offline queue, change-feed pull, three-way merge, conflict copies,
  S3 harness'
plan: thevault-clients
blocked_by:
- '55'
- '59'
status: done
verified: verified
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

### 2026-10-15T07:21:30Z — /work

Claimed.

### 2026-10-15T07:49:29Z — wil

Verified set to verified.

### 2026-10-15T07:49:30Z — wil

Status set to done.

### 2026-10-15T07:49:30Z — wil

Branch ticket/thevault-clients-60, commit d0df92e, based on ticket/thevault-clients-55 (ac07000), which was done but not on main when claimed: merge 55 first.

Design: crates/sync (Engine, Queue, merge). Queue is sync_* tables in the device's state.db on its own rusqlite connection (no state schema bump, to avoid clashing with other tickets' migrations). embedded's LocalHooks.after records every local note write; Writer::apply_remote / apply_remote_files write pulled changes verbatim with no hooks, so they are never queued. Base text comes from the local history via sync_notes.local_version rather than a second copy of every note.

Added server pieces the push needs: PUT /api/notes/{id} + If-None-Match: * (Writer::create_with_id) and PUT /api/assets/{id} (put_asset_with_id), 409 id_taken. Hooks::asset_known lets a device note name an asset only the server holds (Android cache). Writer::evict_asset removes an asset file only.

Conflict-copy link shape (SPEC open question): built-in conflict_of field (schema RESERVED_FIELDS, writer arrange, index link). Web banner 'Conflict copy' in NoteView.

Policies chosen: device delete loses to a server change since its base; device edit to a server-deleted note restores it first; local definition edits are never pushed and are not refused locally. Engine is blocking (reqwest blocking): run off the async runtime. Android asset fetch-on-404 is exposed as Engine::fetch_asset/note_opened, not yet wired into embedded's asset route: the Android ticket wires it.

Evidence: ./dev just check exit 0; ./dev just test-e2e 255 passed, 2 skipped (pre-existing screenshot specs); S3 crates/sync/tests/two_node.rs 8/8; mutation checks (merge forced to clash, bases not reported, handshake skipped) each turned an S3 case red.

### 2026-10-15T07:54:09Z — wil

Rebased onto main 51f9586 (55 had landed as 05fb185); only conflict was the Cargo.toml workspace member list (push + sync, kept both). ./dev just check exit 0 on the rebased branch; conflict-copy.spec.ts passed. main fast-forwarded to 261f4da.
