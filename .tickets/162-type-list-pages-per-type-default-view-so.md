---
id: '162'
title: 'Type list pages: per-type default view, sort and page size; paging; a details
  view; email and message default to details by sent date'
plan: lists
blocked_by: []
status: todo
verified: unknown
---

# Type list pages: per-type default view, sort and page size; paging; a details view; email and message default to details by sent date

Spec: /home/wil/dev/TheVault/.scratch/plans/lists/SPEC.md (L1 to L5).

Add the type-file list settings with schema validation, docs and a type editor section; make the type list page paged by cursor with First, Previous and Next and server-side sort from the type or the user's column choice; add the details view; ship the email and message defaults with an upgrade step that also patches edited files lacking the block.

Acceptance criteria:
- crates/schema tests: the list settings parse and validate (bad view, unknown sort field, page_size outside 10 to 500 are errors).
- crates/writer tests: a fresh vault's email and message types carry the L5 settings; upgrading a vault whose email and message files were edited (a slot added to a template) adds the block and keeps the edit; a file that already has a list block is left alone.
- Web component tests: a type with 120 notes and page_size 50 shows 1 to 50 of 120, Next shows 51 to 100, Previous returns, First returns to page 1; the type's sort is sent to the query; a column click re-queries with that sort; email opens in the details view by default; a remembered user view wins until Reset to type default.
- A Playwright e2e: the email type list opens in details view sorted by sent, newest first, and pages.
- ./dev just check passes.

## Worklog
