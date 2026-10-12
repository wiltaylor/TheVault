---
id: '14'
title: Event handlers and wscript schema migrations
plan: thevault-prd
blocked_by:
- '13'
- 09
- '10'
status: done
verified: verified
---

# Event handlers and wscript schema migrations

Implement step 5 and step 10 of the write path, and wscript migrations.

Scope:
- `before_create`, `before_update`, `before_delete` run synchronously; can modify the note or reject with a message (D9).
- `after_*` handlers run from a durable queue after commit (D9).
- Handler edits re-enter the write path; depth limit and cycle detection (D9, D30).
- `computed` fields are writable only by handlers (D3).
- Unlink and cascade side effects fire update and delete handlers on affected notes (D5).
- Schema changes naming a migration script run it over every note of the type within one commit (D6).
- Handler registration lives in the type file per `docs/vault-format.md`.

Acceptance criteria (S1):
- A before_update handler computing a field sets it on save; a client write to that field is rejected.
- A before_create handler rejecting a save returns its message and creates no file.
- An after_update handler runs after the response and its edit appears in history attributed to the handler.
- Two handlers that update each other's notes stop at the depth limit with an error naming the cycle.
- Deleting a note with unlink referrers runs their update handlers.
- A migration script transforms every note of a type in one commit.

Seam: S1. Decisions D3, D5, D6, D9, D30.

## Worklog

### 2026-10-12T13:22:37Z — /work

Claimed.

### 2026-10-12T13:38:43Z — wil

Verified set to verified.

### 2026-10-12T13:38:43Z — wil

Status set to done.

### 2026-10-12T13:38:43Z — wil

Committed 8d1a6c9 on ticket/thevault-prd-14 (branch also merges ticket/thevault-prd-13 at aae809c, which was done but not yet on main). All six criteria are demonstrated by crates/server/tests/handlers.rs (7 S1 tests); crates/script/tests/handlers.rs covers the durable after_* queue across a restart and staged before_* writes committing all-or-nothing; ./dev just check exits 0. Design: script::Handlers implements writer::Hooks; the server wires it with Writer::with_hooks + Handlers::start. Writer got a separate write lock and releases its data lock during step 5, so handlers can get/query. Before-handler vault writes go through Writer::stage_* into the write in progress (version is null in the result until commit). Migration jobs cannot write other notes. Chain depth MAX_DEPTH=8, error names the cycle. state.db schema is now version 2 (handler_jobs). Failed after_* jobs are kept in state.db but there is no API to list them yet. Hooks::after gained a &Schema parameter. Server main is no longer #[tokio::main]: reqwest's blocking client panics if dropped inside the runtime.

### 2026-10-12T13:38:50Z — wil

Correction: the commit is 75fefc9, not 8d1a6c9.

### 2026-10-12T13:41:12Z — wil

Merged into main at 2b7a974 (fast-forward). Merging main brought in ticket 16's batch, defs and assets write paths. apply_with now takes the data-lock guard by value; commit_definitions re-locks to restore the old schema on failure. batch, templates, taxonomies, canvases, assets and scripts now take the write lock. A batch dry run runs before_* handlers and queues no after_* handlers. ./dev just check exits 0 on the merged tree.
