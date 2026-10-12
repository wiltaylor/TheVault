---
id: '23'
title: 'Query view block: table, cards, list, details, saved views'
plan: thevault-prd
blocked_by:
- '20'
- '12'
status: done
verified: verified
---

# Query view block: table, cards, list, details, saved views

Build the query view block and the basic views (D8, D14, D33).

Scope:
- Query view block with several saved views, each with its own query and view kind; switching views.
- Table, cards (using the type's card template), list, and details (detail template) views.
- Pagination or virtualisation for every view (D33).
- `this` bound to the note that holds the block.

Acceptance criteria (S5):
- A block with a table view and a cards view over the same query switches between them and both show the same notes.
- A Project template block `from task where project == this` shows only that project's tasks.
- A query returning 50,000 notes renders with fewer than 300 result rows or cards in the DOM.
- Saved views persist across reload.

Seam: S5. Decisions D8, D14, D33.

## Worklog

### 2026-10-12T13:45:45Z — /work

Claimed.

### 2026-10-12T13:57:46Z — wil

Verified set to verified.

### 2026-10-12T13:57:46Z — wil

Status set to done.

### 2026-10-12T13:57:46Z — wil

Branch ticket/thevault-prd-23, commit da2681b. S5 tests in web/e2e/query-view.spec.ts, model unit tests in web/src/views/model.test.ts. The seed vault gains project, page and mass (50,000 notes) types, and task gains a project reference field and a card template. Kanban, calendar, timeline, map and canvas view kinds show a 'not available yet' message; they belong to later tickets. The 50k test pages a few pages in rather than to the end: jumping to the end walks every cursor page in order (about 50 full-query requests to a debug server), which made the other e2e tests flaky under parallel load. A 'jump to end' that does not walk every page would need an API change (offset or reverse cursor), not made here.

### 2026-10-12T13:59:45Z — wil

Merged into main at 4a6bf1f after merging main (ticket 21's block editors) into the branch; conflicts in Render.tsx, BlockHolder.tsx and ContentEditor.tsx resolved, just check and all 23 e2e tests green.
