---
id: '11'
title: 'Query language: parser, evaluator, builder model'
plan: thevault-prd
blocked_by:
- '06'
status: done
verified: verified
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

### 2026-10-12T12:55:19Z — /work

Claimed.

### 2026-10-12T13:07:09Z — wil

Verified set to verified.

### 2026-10-12T13:07:09Z — wil

Status set to done.

### 2026-10-12T13:07:09Z — wil

Committed 0e4082c on ticket/thevault-prd-11. S4 tests: crates/query/tests/query.rs (15 tests; table of 51 queries). Assumptions: (1) expressions are parsed by a purpose-built parser mirroring wscript's operator tiers, string escapes and {{ }} braces, not by the wscript compiler, which is a full statically-typed language; (2) where is evaluated in Rust over index.db rows loaded per from-type, not compiled to SQL - ticket 30 (S6 benchmark) should measure this and push filters down to field_values/term_ancestors if the p95 target is missed; (3) linked_from(type, field) is relative to the note being evaluated (the row), and takes bare names or strings; (4) today() is the UTC date unless Context.utc_offset is set; (5) missing values are false in conditions and sort last in either direction; taxonomy fields sort/group by term order; (6) text->model->text compares parsed queries, so the builder reproduces canonical text exactly and query::format() gives canonical text for any query. Dict and content fields are not readable in expressions (they are not in notes.data).

### 2026-10-12T13:08:19Z — wil

Merged into main as 68d7a22 (main merged into branch first, ./dev just check green).
