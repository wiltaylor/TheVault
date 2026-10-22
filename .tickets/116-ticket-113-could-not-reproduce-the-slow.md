---
id: '116'
title: 'Ticket 113 could not reproduce the slow Google connect finish seen on the
  live 0.1.8 server: with FakeGoogle (2 s Gmail delay, or 60 messages and 200 events
  through the shipped scripts) finish answered in about 4 ms; no lock is held across
  a run in finish, status_of or the scheduler. If it recurs, time the token exchange
  and userinfo calls to Google from the server'
blocked_by: []
status: dropped
verified: unknown
---

# Ticket 113 could not reproduce the slow Google connect finish seen on the live 0.1.8 server: with FakeGoogle (2 s Gmail delay, or 60 messages and 200 events through the shipped scripts) finish answered in about 4 ms; no lock is held across a run in finish, status_of or the scheduler. If it recurs, time the token exchange and userinfo calls to Google from the server

## Worklog

### 2026-10-22T01:46:55Z — wil

Triage 2026-10-22: could not be reproduced (ticket 113's investigation), and 113 made the connect finish return at once with spinners and running rows. Dropped; reopen with a fresh report if the connect hangs again.

### 2026-10-22T01:46:55Z — wil

Dropped.
