---
id: '27'
title: Ink block and ink field
plan: thevault-prd
blocked_by:
- '20'
status: done
verified: verified
---

# Ink block and ink field

Build ink input on perfect-freehand (D18, D29).

Scope:
- One Solid ink component used by the ink block and the ink field editor.
- Pointer events for stylus, touch and mouse; pressure where available; eraser; undo.
- Strokes saved in the encoding from `docs/vault-format.md`.

Acceptance criteria (S5):
- Drawing three strokes with the mouse (Playwright) saves three strokes and reloads the same drawing.
- Erasing one stroke saves two.
- Synthetic pointer events with pressure produce varying stroke width.
- An ink field on a type renders the drawing surface on its tab.

Seam: S5. Decisions D18, D29.

## Worklog

### 2026-10-12T13:54:51Z — /work

Claimed.

### 2026-10-12T14:02:33Z — wil

Verified set to verified.

### 2026-10-12T14:02:33Z — wil

Status set to done.

### 2026-10-12T14:02:33Z — wil

Committed ad70511 on ticket/thevault-prd-27. Web only: web/src/ink/{ink.ts,InkSurface.tsx}, wired into Render.tsx (ink block), ContentEditor (Insert drawing), NoteView (ink field via new notes.applyCompound). S5: web/e2e/ink.spec.ts, 4 tests, plus a sketch type in e2e/vault.ts. Rounding of points (2 places / pressure 3) happens in the client; the writer does not round despite docs/vault-format.md saying it does - filed separately. Playwright gotcha: the editor's ink surface sits below the 720px viewport, so tests scrollIntoViewIfNeeded before page.mouse drawing.

### 2026-10-12T14:06:59Z — wil

Merged into main as 02f635d (fast-forward). Main had moved (tickets 17, 22, 23); conflicts in NoteView, notes.ts, ContentEditor, BlockHolder, styles.css and e2e global-setup were all additive and kept both sides. Post-merge: just check exit 0, Playwright 33/33.
