---
id: '23'
title: 'Query view block: table, cards, list, details, saved views'
plan: thevault-prd
blocked_by:
- '20'
- '12'
status: todo
verified: unknown
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
