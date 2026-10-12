---
id: '13'
title: 'script: wscript host, limits and vault host functions'
plan: thevault-prd
blocked_by:
- '07'
- '11'
status: todo
verified: unknown
---

# script: wscript host, limits and vault host functions

Implement the `script` crate: embed wscript safely.

Scope:
- One VM per worker thread on a blocking pool (VMs are not Send).
- Instruction and call-depth limits per run, plus wall-clock timeout and response-size limits enforced by the host (notes/01).
- Never register wscript's `fs` or `process` modules.
- Host functions: run a query, get a note, create/update/delete notes through `writer` (D30), log.
- HTTP host functions exist but are only registered for after-handler and task contexts, and check a domain allowlist (D10, D30). Secrets by name from `state`.

Acceptance criteria:
- A script in an infinite loop stops at the instruction limit with a clear error.
- A script that sleeps in a host call stops at the wall-clock timeout.
- A script calling an `fs` function fails to compile or run.
- A script creates a note through the host API and the note exists with the script named as author in history.
- An HTTP call to a domain outside the allowlist fails; to an allowed domain on a local stand-in server succeeds (S7 stand-in server lives in `testkit`).
- HTTP functions are unavailable in a before-handler context.

Seam: unit tests plus S7's stand-in HTTP server. Decisions D9, D10, D30.

## Worklog
