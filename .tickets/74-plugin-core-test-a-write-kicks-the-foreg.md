---
id: '74'
title: plugin core test a_write_kicks_the_foreground_loop flakes under host load (30
  s deadline, load avg ~54)
blocked_by: []
status: done
verified: verified
---

# plugin core test a_write_kicks_the_foreground_loop flakes under host load (30 s deadline, load avg ~54)

plugins/tauri-plugin-vault-device/tests/core.rs a_write_kicks_the_foreground_loop failed once inside ./dev just android-test with 'no round ran: {"last_error":null,"last_sync_ms":null,"paired":true,"pending":1}' at load average 53.9 with parallel fleet workers; it passed on an immediate rerun of the core tests alone (12.7 s). Seen from ticket 67's worktree, which does not touch the Rust core. Likely the 30 s deadline, or the settle delay of the foreground loop, under CPU starvation.

## Worklog

### 2026-10-15T16:56:28Z — wil

Status set to claimed.

### 2026-10-15T16:56:28Z — wil

Real slowness under disk pressure (rounds up to 24 s); with /tmp on tmpfs the risk drops; deadline raised to 120 s in a shared constant (genuine timeout increase, must outlast the 30 s request timeout). 10 in a row plus 5 whole core.rs runs pass under load.

### 2026-10-15T16:56:29Z — wil

Verified set to verified.

### 2026-10-15T16:56:29Z — wil

Status set to done.
