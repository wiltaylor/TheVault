---
id: '05'
title: 'state: state.db history, trash, tokens, secrets, run logs'
plan: thevault-prd
blocked_by:
- '01'
status: todo
verified: unknown
---

# state: state.db history, trash, tokens, secrets, run logs

Implement the `state` crate over `.vault/state.db` (SQLite).

Scope:
- Version history: every version of every note, with author (session, token name or handler name), timestamp and content. Opaque version token per note (SPEC write path).
- Trash: deleted notes with restore.
- Optional history pruning by age or count, off by default (D25).
- API tokens: name, scope (`read`, `write`, `admin`), hashed secret, created and last-used times (D11).
- Password: Argon2id hash (CLAUDE.md OQ-S3).
- Secrets: encrypted at rest with the key file from CLAUDE.md OQ-S2; read by name, never listed with values (D10).
- Task run records: start, end, outcome, logs (D10).

Acceptance criteria:
- Recording three versions of a note and reading history returns all three in order with authors.
- Restoring a trashed note returns its last content.
- Pruning by count keeps exactly N versions.
- `state.db` opened without the right key file cannot decrypt a secret, and the error says the key is wrong or missing.
- A secret's plaintext never appears in `state.db` bytes (test greps the file).

Seam: unit tests in the crate. Decisions D10, D11, D13, D25.

## Worklog
