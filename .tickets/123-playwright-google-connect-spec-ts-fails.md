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

### 2026-10-18T05:07:35Z — wil

On main at 9d63672 (119 merged) web/e2e/google-connect.spec.ts passed (1 passed, 6.7s) in .scratch/gates/main-119.log. Likely a timing flake: the Gmail run against the fake can finish before the card polls, so it shows Scheduled rather than Syncing. Fix direction: assert Syncing only while a fake delay holds the run (set_gmail_delay), or accept either state.
