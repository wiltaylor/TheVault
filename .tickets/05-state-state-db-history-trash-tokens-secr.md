---
id: '05'
title: 'state: state.db history, trash, tokens, secrets, run logs'
plan: thevault-prd
blocked_by:
- '01'
status: done
verified: verified
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

### 2026-10-12T12:16:06Z — /work

Claimed.

### 2026-10-12T12:20:27Z — wil

Verified set to verified.

### 2026-10-12T12:20:27Z — wil

Status set to done.

### 2026-10-12T12:20:28Z — wil

Implemented on ticket/thevault-prd-05. API: State::open(path, Option<SecretKey>) / open_with_clock (Clock trait, ms since epoch). record_version takes note_type so trash can restore to notes/<type>/. restore_from_trash only removes the trash row and returns content; the writer must rewrite the file and call record_version. key_file_path(vault_id, override, xdg, home) takes env values as args; callers pass std::env::var_os. SecretKey::create_file writes 0600 and refuses to overwrite. Version tokens are 128-bit random hex. Added workspace deps: argon2 0.5, chacha20poly1305 0.10, rusqlite 0.37 (bundled), sha2 0.10, thiserror 2 — the index ticket should reuse the rusqlite entry.
