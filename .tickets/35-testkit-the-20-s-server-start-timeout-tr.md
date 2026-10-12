---
id: '35'
title: 'testkit: the 20 s server start timeout trips under full-gate load (cli a_restored_backup_serves_the_same_notes_and_history)'
blocked_by: []
status: done
verified: verified
---

# testkit: the 20 s server start timeout trips under full-gate load (cli a_restored_backup_serves_the_same_notes_and_history)

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Fix: testkit start timeout defaults to 120 s and reads TESTKIT_START_TIMEOUT_SECS. Acceptance: ./dev just check passes twice in a row with the cli backup test included.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T15:56:30Z — /work

Claimed.

### 2026-10-12T15:58:53Z — wil

Verified set to verified.

### 2026-10-12T15:58:53Z — wil

Status set to done.

### 2026-10-12T15:58:53Z — wil

testkit start timeout is now start_timeout(): 120 s default, TESTKIT_START_TIMEOUT_SECS overrides (whole seconds > 0, else panic naming the variable). Unit tests in crates/testkit/src/lib.rs. ./dev just check passed twice in a row (exit 0 both), cli a_restored_backup_serves_the_same_notes_and_history ok in both. Left alone: the 20 s polling deadlines in crates/server/tests/handlers.rs:394 and crates/script/tests/handlers.rs:150, which wait for handler effects, not server start.
