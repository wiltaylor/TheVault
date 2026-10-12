---
id: '43'
title: 'web: query views do not re-run after a note is saved, so other views show
  stale values'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# web: query views do not re-run after a note is saved, so other views show stale values

Found by the ticket 31 parity audit. QueryView (web/src/views/QueryView.tsx) keys its resource on the query and this only, not on the changes() signal api.ts bumps after a note write. After a kanban drop or a note save, table/list/cards/calendar/timeline/map views on the same page keep the old value until reload; only kanban (its own columns) and canvas query/note nodes refresh. PRD outcome 3 says a kanban drag is visible in every other view of that note. Reported by code reading, not reproduced in a browser.

## Worklog

### 2026-10-12T15:56:14Z — wil

Triage (orchestrator): Fix: after any successful note write from the client, every mounted query view re-runs its query (the api.ts noteWritten hook). Acceptance (S5): editing a note in one view updates a second view of it on the same page without reload.

### 2026-10-12T15:56:14Z — wil

Status set to todo.

### 2026-10-12T18:30:03Z — /work

Claimed.

### 2026-10-12T18:50:23Z — wil

Verified set to verified.

### 2026-10-12T18:50:23Z — wil

Fixed on ticket/thevault-prd-43 (63435d9). createPagedRows takes an optional refresh accessor: when it changes it reloads the pages already held and swaps them in at once (no flash to empty). QueryView (table/list/cards/details/map), kanban columns, Detail's note resource, calendar (refetchEvents) and timeline (reload the window on screen) all follow changes(). Kanban save no longer bumps its own column stamps; the changes() refresh covers it. Proof: e2e date-views 'a kanban drag shows in every other view of the note on the page without a reload' (kanban drag -> table row, details, and a second kanban update; window marker shows no reload), failing before the fix and passing after; unit test pagedRows.test.ts. Calendar/timeline refresh is implemented but not covered by its own e2e. Full e2e under the fleet's parallel load fails 20-30 tests both with and without this change (baseline measured under the same load); --workers=1 runs 62/62 green.

### 2026-10-12T18:50:23Z — wil

Status set to done.

### 2026-10-12T18:57:01Z — wil

Merged into main (96be839) after merging main in; resolved a QueryView.tsx conflict with the new canvas results view by keeping both. just check green after the merge; related e2e green except 'kanban columns page through long columns', which also fails on baseline under load and passes alone (26.7s of its 30s budget).
