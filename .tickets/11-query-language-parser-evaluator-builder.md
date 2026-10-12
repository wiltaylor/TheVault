---
id: '11'
title: 'Query language: parser, evaluator, builder model'
plan: thevault-prd
blocked_by:
- '06'
status: todo
verified: unknown
---

# Query language: parser, evaluator, builder model

Implement the `query` crate per PRD Appendix B (D42).

Scope:
- Parser for the fixed clause order `from`, `where`, `search`, `sort`, `group by`, `select`, `limit`, with wscript expression syntax.
- Evaluator over `index` (SQLite) with `search` on tantivy (D16).
- Host functions: `today()`, `now()`, `days(n)`, `has()`, `under()`, `any()`, `all()`, `linked_from()`. Dot follows a single reference. `this` binding; using `this` with no note fails clearly.
- Cursor pagination; `limit` caps the whole result (D33).
- A structured builder model with text-to-model and model-to-text conversion; text that the model cannot represent is reported as such (D8).

Acceptance criteria (S4):
- A table-driven test runs every clause and every host function against a fixture vault and checks the result set.
- `under(status, "Active")` matches notes with a descendant term.
- Paging through a 1,000-note result in pages of 50 returns every note exactly once.
- For every query in the test table that the model can represent, text to model to text reproduces the original text.
- Parse errors report line and column.

Seam: S4. Decisions D7, D8, D16, D27, D33, D42.

## Worklog
