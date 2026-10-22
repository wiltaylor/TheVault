---
id: '101'
title: 'Note editor: a date_range of = "date_or_datetime" field edits as two raw text
  inputs (FieldInput PairInput passes kind date_or_datetime to ScalarInput); it needs
  an all-day toggle choosing date or datetime inputs for both ends'
blocked_by: []
status: done
verified: verified
---

# Note editor: a date_range of = "date_or_datetime" field edits as two raw text inputs (FieldInput PairInput passes kind date_or_datetime to ScalarInput); it needs an all-day toggle choosing date or datetime inputs for both ends

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:09:49Z — wil

Claimed.

### 2026-10-22T02:25:36Z — wil

Commit edb73b1 on ticket/101: PairInput gets an All day box for of = date_or_datetime (switchAllDay, rangeIsAllDay). Proven by web/src/components/FieldInput.dateRange.test.tsx and web/e2e/date-or-datetime.spec.ts (run alone, passed); ./dev just check passed. Full e2e suite left to the final run.

### 2026-10-22T02:25:36Z — wil

Verified set to verified.

### 2026-10-22T02:25:36Z — wil

Status set to done.
