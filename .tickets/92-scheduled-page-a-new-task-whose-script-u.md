---
id: '92'
title: 'Scheduled page: a new task whose script uses the device module shows ''Saved.''
  because the script is put (and checked) before the task names it, and the device-role
  problem is filtered out by taskProblemsOfScript'
blocked_by: []
status: done
verified: verified
---

# Scheduled page: a new task whose script uses the device module shows 'Saved.' because the script is put (and checked) before the task names it, and the device-role problem is filtered out by taskProblemsOfScript

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:18Z — wil

Claimed.

### 2026-10-22T02:22:04Z — wil

Scheduled save re-reads api.script after putTask so the task role is checked; status 'Saved, but the script does not compile.' plus the device problem show for new (carried to the task page) and existing tasks. New e2e tests: the new-task one failed before the fix; scheduled.spec 15/15 with --repeat-each=3; just check green. Commit 5dff002 on ticket/50.

### 2026-10-22T02:22:05Z — wil

Verified set to verified.

### 2026-10-22T02:22:05Z — wil

Status set to done.
