---
id: '25'
title: Kanban, calendar and timeline views
plan: thevault-prd
blocked_by:
- '23'
status: done
verified: verified
---

# Kanban, calendar and timeline views

Build the date and status views (D7, D29).

Scope:
- Kanban: columns from a single-term taxonomy field; dragging a card writes the field through the API; columns paginate.
- Calendar with FullCalendar's MIT core over date, datetime and date range fields.
- Timeline with vis-timeline over the same fields.

Acceptance criteria (S5):
- Dragging a card to another column changes the note's taxonomy value in its file.
- A kanban over a multi-term field is refused with a clear message.
- Notes with dates appear on the right calendar days; a date range spans its days.
- The timeline shows date-range notes as ranges and single dates as points.
- A 409 on drag shows the conflict view instead of losing the move.

Seam: S5. Decisions D7, D12, D29.

## Worklog

### 2026-10-12T14:05:10Z — /work

Claimed.

### 2026-10-12T14:19:45Z — wil

Verified set to verified.

### 2026-10-12T14:19:45Z — wil

Status set to done.

### 2026-10-12T14:19:45Z — wil

Branch ticket/thevault-prd-25, commit 75f0c42. Evidence: web/e2e/date-views.spec.ts (6 S5 tests, all pass; 18/18 at --repeat-each=3), full e2e 35/35, just check exit 0. Kanban columns are the view's query narrowed client-side (web/src/views/narrow.ts) to field == "term", paged 25 per column; the API has no extra-filter parameter. Drop saves against the version fetched at dragstart because query rows carry no version. FullCalendar pinned to ~6.1.21: v7 is headless and its plugins still peer on 6.1. Calendar/timeline need one date field kind per view: mixing date and date_range under one name across types would make the window condition fail.
