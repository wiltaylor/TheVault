---
id: '28'
title: 'Canvas: native WCL format, renderer and query nodes'
plan: thevault-prd
blocked_by:
- '23'
- '16'
status: done
verified: verified
---

# Canvas: native WCL format, renderer and query nodes

Build the canvas (D15).

Scope:
- Server: canvas content validated per `docs/vault-format.md` through the normal write path.
- Client: infinite canvas with pan and zoom; text, note-card, link and group nodes; edges; query nodes that render query results as cards; viewport saved.
- Canvas as an embedded content block and on a type's template.

Acceptance criteria (S5):
- Creating two nodes and an edge saves a canvas that validates and reloads identically.
- A query node shows the query's notes and updates when a matching note is created.
- A canvas placed on a type template shows on every note of that type.
- Pan and zoom state persists.

Seam: S5 and S1 for validation. Decisions D15.

## Worklog

### 2026-10-12T14:13:54Z — /work

Claimed.

### 2026-10-12T14:30:56Z — wil

Verified set to verified.

### 2026-10-12T14:30:57Z — wil

Status set to done.

### 2026-10-12T14:30:57Z — wil

Committed 25075a0 on ticket/thevault-prd-28. Server validation and the canvas endpoints already existed (tickets 03/16); this ticket added S1 tests (crates/server/tests/canvas.rs) and the whole client: web/src/canvas/{canvas.ts,CanvasSurface.tsx,CanvasNodes.tsx}. Assumptions: (1) query nodes go live by polling every 3 s, on window focus, and after every note write this client makes (web/src/changes.ts) — the server has no push channel; (2) pan/zoom is saved with the note when editing; in read mode the view moves but is not saved, to avoid a new note version on every pan; (3) canvas cards use plain <a> links, because the router's <A> throws inside a TipTap node view (outside the router). Not built: editing ink-node strokes in place (ink nodes render, and their strokes survive edits), choosing edge sides in the UI (from_side/to_side render when present). Gate: ./dev just check exit 0; Playwright 39 passed.

### 2026-10-12T14:37:20Z — wil

Merged into main as 8ecaf48 (fast-forward), after merging main in. Conflicts resolved: main had moved TemplateBody/NoteLink to web/src/views/parts.tsx (CanvasNodes now imports from there), plus styles.css and e2e/global-setup.ts were combined. Post-merge: just check exit 0; full e2e under parallel load failed a different set of non-canvas specs each run (7, then 4), and all of them passed with --workers=1.
