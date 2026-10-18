---
id: '113'
title: 'Integrations Google card gives no feedback while connecting or syncing: add
  a spinner on Connect and Save, show running state per side, and never make the finish
  request wait on the first runs'
blocked_by: []
status: todo
verified: unknown
---

# Integrations Google card gives no feedback while connecting or syncing: add a spinner on Connect and Save, show running state per side, and never make the finish request wait on the first runs

Found on the live server (0.1.8), 2026-10-18: after pasting the loopback URL, the page showed nothing for a long time ('it just hangs'), then the connection turned out to have worked (connected, both tasks unpaused, first runs started 03:55:41Z). The Gmail first run took 36 s and the Calendar first run over 6 minutes (210 event notes and counting), with no sign on the card that anything was happening.

Build:
1. Connect Google, the paste submit, Save settings, Run now and Disconnect show a spinner and disable their button while their request is in flight, and show an error inline if it fails.
2. POST /api/integrations/google/finish answers as soon as the secrets are stored and the tasks unpaused; it never waits on the first runs (check crates/api/src/google.rs finish and status_of: find what made the request slow on the live server, e.g. a lock held by a running task, the token exchange, or status_of waiting on scheduler state, and fix the cause; add a test that finish answers while a slow task run is in progress, using the fake Google with an artificial delay).
3. GET /api/integrations/google reports  per side while a run is in progress (from the scheduler), and the card shows 'Syncing…' with a spinner for that side, polling every few seconds while either side runs, then the last run time and the run's summary line (for example 'Gmail: 0 new, 0 relabelled, 0 archived').
4. After a successful finish the card switches straight to the connected state with both sides showing 'Syncing…'.

Acceptance criteria: web component tests for each spinner state and the running rows; server test that finish answers within 1 s while a slow run holds the task; server test of running true/false in GET; ./dev just check passes.

## Worklog
