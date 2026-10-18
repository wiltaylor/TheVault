---
id: '108'
title: Shipped task files that Connect Google unpauses (ticket 96) then differ from
  every shipped hash, so a later upgrade of tasks/google_gmail.wcl or google_calendar.wcl
  is kept and the user is told a newer version exists; paused state may need to live
  outside the shipped file or be ignored by the hash check
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# Shipped task files that Connect Google unpauses (ticket 96) then differ from every shipped hash, so a later upgrade of tasks/google_gmail.wcl or google_calendar.wcl is kept and the user is told a newer version exists; paused state may need to live outside the shipped file or be ignored by the hash check

## Worklog

### 2026-10-17T23:58:31Z — wil

Same defect as ticket 107 (unpausing a shipped task rewrites the file so it stops matching shipped hashes); work it there.

### 2026-10-18T00:32:27Z — wil

Ticket 107 fixes this (branch ticket/107): the shipped-file comparison now ignores a task file's paused field.
