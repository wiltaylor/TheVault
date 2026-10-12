---
id: '24'
title: Visual query builder
plan: thevault-prd
blocked_by:
- '23'
status: todo
verified: unknown
---

# Visual query builder

Build the visual builder that edits the same text query (D8).

Scope:
- Builder UI for every construct the `query` builder model supports.
- Text mode and builder mode switch both ways; a query the model cannot represent opens in text mode only and is never rewritten.

Acceptance criteria (S5):
- Building a query with a type, two conditions, a sort and a limit produces the expected text.
- Opening agent-written query text in the builder and saving without changes leaves the text byte-identical.
- A query the model cannot represent opens in text mode with a notice.

Seam: S5 plus S4 round-trip tests. Decisions D8, D27, D42.

## Worklog
