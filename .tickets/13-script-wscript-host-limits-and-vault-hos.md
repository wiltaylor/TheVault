---
id: '13'
title: 'script: wscript host, limits and vault host functions'
plan: thevault-prd
blocked_by:
- '07'
- '11'
status: done
verified: verified
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

### 2026-10-12T13:13:00Z — /work

Claimed.

### 2026-10-12T13:22:06Z — wil

Verified set to verified.

### 2026-10-12T13:22:06Z — wil

Status set to done.

### 2026-10-12T13:22:06Z — wil

Committed a8a4219 on ticket/thevault-prd-13. All six criteria are demonstrated by crates/script/tests/host.rs (17 tests), and ./dev just check exits 0. Design: Host::new(Arc<dyn Vault>, workers, Limits) and Host::run(Job) -> Outcome{result, log}. Job.kind (Before/After/Task/Migration) picks one of two wscript Contexts; http and secrets exist only for After and Task, so using them elsewhere fails to compile. Notes in and out of scripts use the API's JSON form {id,type,version,note:{fields,blocks}}. The author is Author::Handler(job.name). For ticket 14: the Vault impl for Writer calls Writer::create/update/delete, which take the writer mutex. A before_* handler running inside Writer::apply that calls vault::create will therefore deadlock. Ticket 14 must make that re-entry work, for example with a Vault impl that queues the change into the write in progress. Waiting on Host::run from inside a running script also needs care: the pool is fixed-size, so nested runs can starve it. reqwest gained the rustls-tls feature at the workspace level so https works.

### 2026-10-12T13:23:03Z — wil

Merged into main as aae809c, after merging main (tickets 09 and 10) into the branch. ./dev just check exited 0.
