---
id: '25'
title: Kanban, calendar and timeline views
plan: thevault-prd
blocked_by:
- '23'
status: todo
verified: unknown
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
