---
id: '181'
title: Calendar invites and changes through the Outbox, with attendees emailed
plan: outbox
blocked_by:
- '180'
status: done
verified: verified
---

# Calendar invites and changes through the Outbox, with attendees emailed

Spec: /home/wil/dev/TheVault/.scratch/plans/outbox/SPEC.md, X5. Acceptance: tests against the fake Calendar that an approved new event item creates the event note and the insert carries sendUpdates=all; a change item patches only its non-empty fields with sendUpdates=all; a direct user edit still sends no sendUpdates; send_updates is cleared after write_back; the item becomes sent with sent_ref and result, or failed with Google's message. Targeted tests only.

## Worklog

### 2026-10-23T09:43:00Z — wil

Verified set to verified.

### 2026-10-23T09:43:00Z — wil

Status set to done.
