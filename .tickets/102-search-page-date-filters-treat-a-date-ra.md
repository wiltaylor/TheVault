---
id: '102'
title: Search page date filters treat a date_range of = "date_or_datetime" field as
  days (searchModel rangeCond days = of !== datetime), so on/before/after compare
  timed values against UTC midnights rather than local days
blocked_by: []
status: done
verified: verified
---

# Search page date filters treat a date_range of = "date_or_datetime" field as days (searchModel rangeCond days = of !== datetime), so on/before/after compare timed values against UTC midnights rather than local days

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:09:49Z — wil

Claimed.

### 2026-10-22T02:25:36Z — wil

Commit edb73b1 on ticket/101: searchModel mixedRangeCond compares timed ends against local midnights and dated ends as whole days (west of UTC with a by-name date test); in the last n days goes by local days. Proven by the evaluator test in searchModel.test.ts across UTC, +11 and -5, and web/e2e/date-or-datetime.spec.ts against the real query engine (run alone, passed); ./dev just check passed. Full e2e suite left to the final run.

### 2026-10-22T02:25:36Z — wil

Verified set to verified.

### 2026-10-22T02:25:36Z — wil

Status set to done.
