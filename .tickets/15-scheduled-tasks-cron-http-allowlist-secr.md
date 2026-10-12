---
id: '15'
title: 'Scheduled tasks: cron, HTTP allowlist, secrets, retries, clock'
plan: thevault-prd
blocked_by:
- '14'
status: todo
verified: unknown
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
