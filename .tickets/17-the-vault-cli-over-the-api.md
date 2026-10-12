---
id: '17'
title: The vault CLI over the API
plan: thevault-prd
blocked_by:
- '16'
- '15'
status: todo
verified: unknown
---

# The vault CLI over the API

Implement the `vault` binary as an API client (D32).

Scope:
- Config: server URL and token from flags or environment.
- JSON output on every command.
- Commands: note get/put/delete/history/restore, raw WCL read and write, batch apply with `--dry-run`, upsert, asset upload, query, search, schema inspect, taxonomy and type management, task run/history, token management.
- Local commands: `vault init [--starter]` (layout, key file, first password per CLAUDE.md OQ-S2/S3), `vault passwd`, `vault reindex`, `vault backup [--include-key]` (D25, D28).
- `--starter` content is ticket `starter`; here `--starter` may fail with "not yet available" only until that ticket lands, and that ticket must remove the message.

Acceptance criteria (S2):
- Every command prints valid JSON (a test runs each and parses it).
- `vault put` of WCL then `vault get` returns the canonical form.
- `vault batch --dry-run` output equals the real run's output.
- `vault reindex` on a vault with one broken file reports that file and exits non-zero.
- `vault backup` then restoring into a new folder serves the same notes and history.
- `vault init` creates the layout and key file and sets the password from `--password-stdin`.

Seam: S2. Decisions D25, D28, D32.

## Worklog
