---
id: '97'
title: 'Gmail sync task: labelled or all mail into email and email_thread notes, history
  cursor, back-fill, attachments'
blocked_by:
- '95'
status: todo
verified: unknown
---

# Gmail sync task: labelled or all mail into email and email_thread notes, history cursor, back-fill, attachments

The spec is /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (gitignored, in the main checkout; read it from that absolute path, since worktrees do not have .scratch). Decision ids D1-D20 there are the google-sync map's, at /home/wil/dev/TheVault/.scratch/plans/google-sync/MAP.md, with reasoning in its notes/; cite them in commits as 'google-sync Dn'. Research on Google's OAuth and API behaviour: /home/wil/dev/TheVault/.scratch/plans/google-sync/notes/01-google-oauth-constraints.md.

Scope: the 'Gmail task' section of the spec and the parts of 'Secrets, tasks, scripts' and the invalid_grant rule that apply to google_gmail. Ship tasks/google_gmail.wcl (paused) and scripts/google_gmail.ws as shipped defaults with hashes. Extend the fake Google server's gmail module.

Acceptance criteria (server tests through TestServer and POST /api/tasks/google_gmail/run against the fake):
- Labels mode imports only the configured labels' messages inside the window; all mode imports every message in the window.
- A second run with no new mail creates nothing; a label change updates the email's labels only; a deleted message archives its note.
- A history 404 falls back to a full list without duplicating.
- Back-fill pages older mail over successive runs and stops at gmail_backfill_to.
- text/plain body, HTML-only body converted, html asset stored, attachment under 700 KB stored, larger one listed, an over-1 MiB message stored as headers and snippet.
- Thread grouping and participants; sender linked to a person by emails, unknown sender left raw.
- invalid_grant: one bell notification across two runs, last_gmail_error set, the run is not recorded as failed.
- ./dev just check passes.

## Worklog
