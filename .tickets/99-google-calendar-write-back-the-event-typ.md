---
id: '99'
title: 'Google Calendar write-back: the event type''s google_event handler creates,
  patches with If-Match, deletes and recreates on restore'
blocked_by:
- '98'
status: claimed
verified: unknown
---

# Google Calendar write-back: the event type's google_event handler creates, patches with If-Match, deletes and recreates on restore

The spec is /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (gitignored, in the main checkout; read it from that absolute path, since worktrees do not have .scratch). Decision ids D1-D20 there are the google-sync map's, at /home/wil/dev/TheVault/.scratch/plans/google-sync/MAP.md, with reasoning in its notes/; cite them in commits as 'google-sync Dn'. Research on Google's OAuth and API behaviour: /home/wil/dev/TheVault/.scratch/plans/google-sync/notes/01-google-oauth-constraints.md.

Scope: the 'Write-back handler' section of the spec. Declare the handler on the event type (after_create, after_update, after_delete, allow_domains and secrets as the spec says), ship scripts/google_event.ws with its hash. Use the snapshot and etag format the calendar ticket documented.

Acceptance criteria (server tests through TestServer, vault edits through the notes API, observed as requests the fake received):
- Editing an event note's title PATCHes only the title with If-Match and stores the new etag and snapshot; the follow-up after_update sends nothing (loop guard).
- A task-written change (simulated by running google_calendar after a Google-side edit) sends nothing back.
- A 412 merges field by field and retries once.
- Creating an event note inserts it into its calendar or default_calendar and stores google_ref.
- Deleting deletes in Google; restoring a trashed note recreates it with a new google_ref.
- Editing an occurrence patches only that instance.
- ./dev just check passes.

## Worklog

### 2026-10-17T23:52:28Z — /work

Claimed.
