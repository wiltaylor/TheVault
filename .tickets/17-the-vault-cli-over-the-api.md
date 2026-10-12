---
id: '17'
title: The vault CLI over the API
plan: thevault-prd
blocked_by:
- '16'
- '15'
status: done
verified: verified
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

### 2026-10-12T13:52:13Z — /work

Claimed.

### 2026-10-12T14:03:22Z — wil

Verified set to verified.

### 2026-10-12T14:03:23Z — wil

Status set to done.

### 2026-10-12T14:03:23Z — wil

Branch ticket/thevault-prd-17 @ d4511e0. S2 tests in crates/cli/tests/cli.rs (10 tests). Assumptions: (1) vault id is a random hex stored in a new state.db meta table (schema v3), so it travels with backups; (2) vault-server now loads the key from $XDG_CONFIG_HOME/thevault/<id>.key when VAULT_SECRET_KEY_FILE is unset; (3) backup archive is SQLite Archive (sqlar), uncompressed, restored with 'vault restore-backup' (no new crates needed); (4) 'batch --dry-run output equals the real run' is compared after normalising what a dry run cannot know: versions, ids of created notes, created/updated stamps, and the dry_run/committed flags (the API from ticket 16 returns version null on dry runs by design); (5) reindex works on the folder directly and the server should be stopped first. 'vault commands' lists every command; the every-command test fails if a new command is not exercised. Ticket 18 must remove the --starter 'not yet available' error in crates/cli/src/local.rs.

### 2026-10-12T14:04:35Z — wil

Merged into main at 452221c (fast-forward after merging main into the branch; ./dev just check green).
