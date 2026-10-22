---
id: '115'
title: Run now on the Integrations Google card (POST /api/tasks/{name}/run) holds
  its request open for the whole run, minutes on a first Calendar sync; consider answering
  once the run starts and letting the card poll running
blocked_by: []
status: done
verified: verified
---

# Run now on the Integrations Google card (POST /api/tasks/{name}/run) holds its request open for the whole run, minutes on a first Calendar sync; consider answering once the run starts and letting the card poll running

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:42Z — wil

Claimed.

### 2026-10-22T02:04:49Z — wil

Done on ticket/109 (1759518): POST /api/tasks/{name}/run answers 202 with the running run via trigger_in_background (now returns the run record); ?wait=true keeps the old 200; vault task run waits by default, --no-wait added; web Integrations card and Scheduled page poll GET runs/{id}.

### 2026-10-22T02:04:50Z — wil

Verified set to verified.

### 2026-10-22T02:04:50Z — wil

Status set to done.
