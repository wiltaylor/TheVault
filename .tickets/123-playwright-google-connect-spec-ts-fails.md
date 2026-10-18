---
id: '123'
title: 'Playwright google-connect.spec.ts fails: after Finish connecting the Gmail
  row shows Scheduled, not Syncing'
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# Playwright google-connect.spec.ts fails: after Finish connecting the Gmail row shows Scheduled, not Syncing

Seen 2026-10-18 while verifying ticket 119: ./dev just web-e2e-test had 309 passed, 1 failed, and a rerun of e2e/google-connect.spec.ts alone failed the same way at line 98: expect(card.getByRole('row', { name: 'Gmail' })).toContainText('Syncing') received 'GmailScheduledNeverNo errorsRun Gmail now'. Ticket 119 touches templates, the query parser and the note page, not integrations; not yet checked on main.

## Worklog
