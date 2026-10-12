---
id: '14'
title: Event handlers and wscript schema migrations
plan: thevault-prd
blocked_by:
- '13'
- 09
- '10'
status: todo
verified: unknown
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
