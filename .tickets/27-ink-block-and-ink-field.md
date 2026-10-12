---
id: '27'
title: Ink block and ink field
plan: thevault-prd
blocked_by:
- '20'
status: todo
verified: unknown
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
