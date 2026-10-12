---
id: '20'
title: 'Block editor core: TipTap, Solid node views, text and inline markup'
plan: thevault-prd
blocked_by:
- '19'
status: done
verified: verified
---

# Block editor core: TipTap, Solid node views, text and inline markup

Build the editor foundation (D29, D34, D41).

Scope:
- TipTap on ProseMirror with an adapter that renders Solid components inside node views (handle stopEvent and ignoreMutation).
- Text blocks: headings, paragraphs, lists, quotes.
- Inline marks per PRD Appendix A: bold, italic, underline, strike, code, highlight, palette colour, link, note mention (`[[ULID]]` with live title, `[[ULID|label]]`).
- Mention picker searching notes.
- Serialise the editor document to and from the content-field structure of `docs/vault-format.md` through the API.

Acceptance criteria:
- Vitest: every inline mark round-trips text to editor to text unchanged; a table of tricky escapes round-trips.
- Playwright: typing each mark through keyboard shortcuts or toolbar saves the expected WCL (checked via API).
- Playwright: renaming a mentioned note changes the rendered mention without changing the mentioning note's file.
- A Solid component inside a node view keeps focus and input without ProseMirror stealing events (Playwright).

Seam: S5 plus Vitest. Decisions D29, D34, D41.

## Worklog

### 2026-10-12T13:26:43Z — /work

Claimed.

### 2026-10-12T13:43:36Z — wil

Verified set to verified.

### 2026-10-12T13:43:36Z — wil

Status set to done.

### 2026-10-12T13:43:36Z — wil

Branch ticket/thevault-prd-20, commit a120575. Ticket 19's branch (ba08afb) was not on main at claim time, so it is merged into this branch (34ecc3d); conflicts in crates/api/src/{lib,types}.rs resolved by keeping both sides (19's GET /api/types plus main's type change/delete/flagged routes). Evidence: ./dev just check exit 0 (vitest 103 passed incl. 92 markup/editor round-trip cases); ./dev just web-e2e-test 14 passed; editor.spec.ts --repeat-each 15: 60 passed. Mutation check: with stopEvent forced to false the focus test fails. Design: inline delimiters toggle marks (run of * length 1/2/3 = italic/bold/both); code closes at every other mark change; +,~,= escaped only when doubled or at a run edge. Trailing empty paragraphs are dropped on save. Left alone: shell.spec.ts's type-list test (ticket 19) fails under --repeat-each because the new row falls outside the virtualised list once many tasks exist; it passes in a normal run.

### 2026-10-12T13:45:09Z — wil

Merged into main at 61f7522 (fast-forward). main was merged in first; the crates/api conflicts took main's resolution. ./dev just check exit 0, web-e2e-test 14 passed.
