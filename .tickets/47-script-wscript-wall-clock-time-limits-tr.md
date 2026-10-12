---
id: '47'
title: 'script: wscript wall-clock time limits trip under full-gate load (handlers
  and host tests)'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# script: wscript wall-clock time limits trip under full-gate load (handlers and host tests)

Found while gating ticket 41 on a host at load average 10-13 with other fleet workers running. Three failures over three runs, all in crates/script, none in code ticket 41 touched:

- ./dev just check: script/tests/handlers.rs after_jobs_queued_when_the_server_stops_run_when_it_starts_again panicked at handlers.rs:159 'the queue never emptied'; stdout: 'after_* handler job 1 failed: script "review" stopped: it ran longer than its 30s time limit'.
- ./dev cargo test --workspace --no-fail-fast: script/tests/host.rs:200 a_script_creates_a_note_recorded_as_made_by_the_script unwrap on Err(Timeout { script: "rollup", limit: 10s }).
- ./dev cargo test -p script alone: the handlers test failed again (117 s run).

The limits are wall-clock, so a loaded host stops scripts that are not runaway. Same shape as ticket 35 (testkit start timeout). Options: tests set a generous Limits/timeout, or the limit measures CPU time rather than wall time (D10 says what the limit means; check before changing it).

## Worklog

### 2026-10-12T19:24:28Z — wil

Triage (orchestrator): the clean run on an idle machine passes ./dev just check, so this is load sensitivity in tests. Fix: tests that are not about time limits use a generous wall-clock limit (e.g. 30 s) so contention cannot trip them; the dedicated timeout tests keep tight limits but assert on the limit error, not elapsed time. Product defaults stay as specified. Acceptance: ./dev just check passes while a parallel CPU and I/O load runs (e.g. stress-ng or a concurrent cargo build in another worktree), recorded in the worklog.

### 2026-10-12T19:24:28Z — wil

Status set to todo.

### 2026-10-12T19:33:33Z — /work

Claimed.

### 2026-10-12T19:45:33Z — wil

Verified set to verified.

### 2026-10-12T19:45:33Z — wil

Fix in 1c5b238 (crates/script/tests only; Limits::default and server unchanged). host.rs: fixture uses roomy() = defaults with a 600 s timeout; a_script_creates_a_note job timeout 10 s -> 600 s. The sleep-in-HTTP timeout test now asserts on Error::Timeout plus the run log 'asking\n' (Host::run's GRACE fallback returns an empty log, so this proves the HTTP call ended at the deadline without timing); mutation check: multiplying the HTTP request timeout by 100 makes it fail on that log assertion. Parallel-workers test: 3 s replies, assert < 5.5 s (serial is 6 s). handlers.rs: Handlers limits 600 s, queue poll deadline 20 s -> 300 s. Could not reproduce the original stall with synthetic load alone: at load avg 45 (2 busy loops per core + 4 fsync dd writers) the script tests still finished in ~2 s, so the reported 30 s+ stalls were probably writeback/fsync stalls from many concurrent cargo builds. Acceptance: ./dev just check exit 0 in 133 s while the busy loops, fsync writers and a concurrent CARGO_TARGET_DIR=scratch cargo build --workspace --release ran, load average 39 at start and 82 at end.

### 2026-10-12T19:45:33Z — wil

Status set to done.
