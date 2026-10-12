---
id: '20'
title: 'Block editor core: TipTap, Solid node views, text and inline markup'
plan: thevault-prd
blocked_by:
- '19'
status: todo
verified: unknown
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
