---
id: '15'
title: 'Scheduled tasks: cron, HTTP allowlist, secrets, retries, clock'
plan: thevault-prd
blocked_by:
- '14'
status: done
verified: verified
---

# Scheduled tasks: cron, HTTP allowlist, secrets, retries, clock

Implement the `scheduler` crate and task API.

Scope:
- Task files define cron, script, domain allowlist and readable secret names (D10).
- Clock interface; a `--test-clock` server flag that only test builds accept, controllable over a test-only endpoint (S7).
- Run records with logs in `state`; failed runs retry with backoff; latest-failure status exposed (D10).
- API: task CRUD (admin), run history, trigger now, secrets CRUD (admin; values write-only).

Acceptance criteria (S1 + S7):
- Advancing the test clock past a cron time runs the task exactly once.
- A task calling the stand-in HTTP server with a secret header succeeds and updates a note.
- A task calling a non-allowlisted domain fails and the run record says why.
- A failing task retries with increasing delays and the latest run shows failed.
- The release build of the server rejects `--test-clock`.
- The secrets API never returns a secret value.

Seam: S1 and S7. Decisions D10.

## Worklog

### 2026-10-12T13:39:21Z — /work

Claimed.

### 2026-10-12T13:50:17Z — wil

Verified set to verified.

### 2026-10-12T13:50:17Z — wil

Status set to done.

### 2026-10-12T13:50:17Z — wil

Committed 4090bb5 on ticket/thevault-prd-15. Evidence: crates/server/tests/scheduler.rs (8 S1+S7 tests) all pass; release check run with 'cargo test --release -p server --test scheduler only_test_builds' and by running target/release/vault-server --test-clock (exits 1: '--test-clock is accepted only by test builds'). Built on main WITHOUT ticket 14: merging ticket/thevault-prd-14 conflicts with ticket 16's writer changes, so 14's merge is left to its own worker. Expect conflicts in crates/server/src/main.rs when both land: 14 adds script::Handlers + a blocking main; this branch also moved to a blocking main with a script::Host for the scheduler. Unify to one Host when merging if Handlers exposes one. Secret key: server reads VAULT_SECRET_KEY_FILE only; the XDG default path needs a vault id, which ticket 17 (vault init) defines. /api/test/clock is registered only with --test-clock and is not in the endpoint listing. Retry state lives in memory, so a restart drops a waiting retry.

### 2026-10-12T13:51:48Z — wil

Merged into main at 413b5d5 (fast-forward). Resolved server main.rs against 14: Handlers hooks plus a separate scheduler Host. just check green after merge.
