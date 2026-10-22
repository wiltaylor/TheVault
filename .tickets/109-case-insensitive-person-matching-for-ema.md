---
id: '109'
title: 'Case-insensitive person matching for email addresses: the query language''s
  has() compares strings exactly, so google_gmail.ws tries the lower-case address
  and the address as sent; a person storing a mixed-case address that matches neither
  is not linked (google-sync D5)'
blocked_by: []
status: done
verified: verified
---

# Case-insensitive person matching for email addresses: the query language's has() compares strings exactly, so google_gmail.ws tries the lower-case address and the address as sent; a person storing a mixed-case address that matches neither is not linked (google-sync D5)

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:47:42Z — wil

Claimed.

### 2026-10-22T02:04:49Z — wil

Done on ticket/109 (1759518): has_ci(list, value) in the query language (evaluator, SQL via vault_same, web when.ts, shared when_cases.json), used by google_gmail.ws, google_calendar.ws and google_event.ws; google_gmail test stores Sam@Example.COM and fails on the old script.

### 2026-10-22T02:04:49Z — wil

Verified set to verified.

### 2026-10-22T02:04:50Z — wil

Status set to done.
