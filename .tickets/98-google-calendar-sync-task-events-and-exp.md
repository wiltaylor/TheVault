---
id: '98'
title: 'Google Calendar sync task: events and expanded series occurrences into event
  notes, sync tokens, window roll, field-by-field merge'
blocked_by:
- '94'
- '95'
status: done
verified: verified
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

### 2026-10-17T23:51:18Z — wil

Verified set to verified.

### 2026-10-17T23:51:18Z — wil

Status set to done.

### 2026-10-17T23:51:19Z — wil

Branch ticket/98, commit dd88d7c. Lead's change: event.calendar is now a string holding the Google calendar id (a script cannot change a type, so no select options); the first criterion's 'fills the calendar select options' was dropped. Tests: crates/server/tests/google_calendar.rs (8, through TestServer on --test-clock 2026-03-01 against FakeGoogle) and crates/testkit/tests/fake_calendar.rs (4). Snapshot/etag format for ticket 99 is in docs/scripts.md 'The Google Calendar sync' > 'Snapshot and etag'. The task does not call calendarList (the fake serves it). Event descriptions are stored as paragraphs unescaped, so Google text with inline-markup characters renders as markup, and HTML descriptions are not converted. Shipped-file list grew, so tests that listed every task or script (embedded handle.rs, server scheduler.rs and shipped_defaults.rs, writer shipped_defaults.rs) were updated. Tickets 96/97 will conflict in SHIPPED_FILES and docs/scripts.md tail; resolve by keeping both.
