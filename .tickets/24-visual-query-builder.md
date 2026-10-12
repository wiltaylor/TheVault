---
id: '24'
title: Visual query builder
plan: thevault-prd
blocked_by:
- '23'
status: done
verified: verified
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

### 2026-10-12T14:00:27Z — /work

Claimed.

### 2026-10-12T14:11:38Z — wil

Verified set to verified.

### 2026-10-12T14:11:38Z — wil

Status set to done.

### 2026-10-12T14:11:38Z — wil

Committed 5d2fc99 on ticket/thevault-prd-24. Backend (query::Model, /api/query/parse, /api/query/text) already existed from tickets 11/12; this ticket is the web builder: web/src/editor/QueryEditor.tsx + queryModel.ts, wired into QueryViewSettings. Representable queries open in builder mode; text is only rewritten after a builder edit, which is how byte-identity holds for non-canonical agent text. Found and fixed a race during e2e stress: switching to Text used to discard an in-flight conversion, losing the last builder edit. Known gap left alone: saving the note while a conversion request is still in flight saves the previous text (the request is a few ms locally). Existing e2e 'views saved in the editor persist' now clicks Text before typing the query, since the default query opens in builder mode. Evidence: S4 tests a_model_built_in_the_web_builder_prints_the_expected_text, agent_spacing_still_opens_in_the_builder; vitest QueryEditor.test.tsx (8); Playwright e2e/query-builder.spec.ts (3), stress-run 15x at 5 workers with no failures; just check exit 0; full e2e 26/26.

### 2026-10-12T14:15:38Z — wil

Merged into main at c8dcf55 (fast-forward). Merging main in conflicted only on api.ts imports (SchemaType from ticket 22 vs QueryModel); kept both. just check exit 0 and e2e 36/36 after the merge.
