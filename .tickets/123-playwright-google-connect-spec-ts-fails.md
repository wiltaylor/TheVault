---
id: '123'
title: 'Playwright google-connect.spec.ts fails: after Finish connecting the Gmail
  row shows Scheduled, not Syncing'
blocked_by: []
status: done
verified: verified
---

# Playwright google-connect.spec.ts fails: after Finish connecting the Gmail row shows Scheduled, not Syncing

Seen 2026-10-18 while verifying ticket 119: ./dev just web-e2e-test had 309 passed, 1 failed, and a rerun of e2e/google-connect.spec.ts alone failed the same way at line 98: expect(card.getByRole('row', { name: 'Gmail' })).toContainText('Syncing') received 'GmailScheduledNeverNo errorsRun Gmail now'. Ticket 119 touches templates, the query parser and the note page, not integrations; not yet checked on main.

## Worklog

### 2026-10-18T05:07:35Z — wil

On main at 9d63672 (119 merged) web/e2e/google-connect.spec.ts passed (1 passed, 6.7s) in .scratch/gates/main-119.log. Likely a timing flake: the Gmail run against the fake can finish before the card polls, so it shows Scheduled rather than Syncing. Fix direction: assert Syncing only while a fake delay holds the run (set_gmail_delay), or accept either state.

### 2026-10-18T05:16:27Z — /work

Claimed.

### 2026-10-18T05:39:49Z — wil

Verified set to verified.

### 2026-10-18T05:39:49Z — wil

Status set to done.

### 2026-10-18T05:39:49Z — wil

Fixed on ticket/123 at d16c3b4. Root cause: the spec ran a no-op script, so the first run could end before finish's own status read; the answer then showed Scheduled and the card never polled. The spec now runs the shipped scripts against the fake with gmail-delay/calendar-delay held (new stdin commands on testkit-fake-google, acked per line), asserts Syncing, releases, and asserts 'Gmail: 0 new' and '0 created, 0 updated, 0 trashed'. The Calendar row had the same race, so the fake gained set_calendar_delay. Proof: --repeat-each=10 --workers=1 gave 10 passed; full e2e 310 passed, 4 skipped; ./dev just check green. Note: --repeat-each without --workers=1 fails 7/10 because parallel copies share the one global e2e server's Google config (Malformed auth code); that is test isolation, not a product bug.
