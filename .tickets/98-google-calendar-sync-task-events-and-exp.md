---
id: '98'
title: 'Google Calendar sync task: events and expanded series occurrences into event
  notes, sync tokens, window roll, field-by-field merge'
blocked_by:
- '94'
- '95'
status: claimed
verified: unknown
---

# Google Calendar sync task: events and expanded series occurrences into event notes, sync tokens, window roll, field-by-field merge

The spec is /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (gitignored, in the main checkout; read it from that absolute path, since worktrees do not have .scratch). Decision ids D1-D20 there are the google-sync map's, at /home/wil/dev/TheVault/.scratch/plans/google-sync/MAP.md, with reasoning in its notes/; cite them in commits as 'google-sync Dn'. Research on Google's OAuth and API behaviour: /home/wil/dev/TheVault/.scratch/plans/google-sync/notes/01-google-oauth-constraints.md.

Scope: the 'Calendar task' section of the spec and the parts of 'Secrets, tasks, scripts' and the invalid_grant rule that apply to google_calendar. Ship tasks/google_calendar.wcl (paused) and scripts/google_calendar.ws as shipped defaults with hashes. Extend the fake Google server's calendar module. Write the snapshot and etag format down in docs/scripts.md or docs/google.md, since the write-back ticket depends on it.

Acceptance criteria (server tests through TestServer and POST /api/tasks/google_calendar/run against the fake):
- First run fills the calendar select options and imports single events, all-day as dates and timed as datetimes with offsets.
- A recurring series becomes one note per occurrence inside -30 to +365 days with series text; a modified instance updates its note; a cancelled event or instance trashes its note.
- An incremental run uses the sync token; a 410 resyncs without duplicating.
- The daily window roll adds the new day's occurrences.
- A note changed in the vault and in Google merges field by field (D10).
- invalid_grant: one bell notification across two runs, last_calendar_error set, run not recorded as failed.
- ./dev just check passes.

## Worklog

### 2026-10-17T23:29:54Z — /work

Claimed.
