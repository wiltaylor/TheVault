---
id: '42'
title: 'web: the canvas query view kind renders ''not available yet'''
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# web: the canvas query view kind renders 'not available yet'

Found by the ticket 31 parity audit. PRD 'Query view' and SPEC 'Web client' list canvas as a query view kind alongside table, cards, list, details, kanban, calendar, timeline and map. web/src/views/QueryView.tsx:274 shows a placeholder for kind = canvas instead of rendering the result on a canvas. Canvas blocks with query nodes do work; only the query-view kind is missing.

## Worklog

### 2026-10-12T15:56:14Z — wil

Triage (orchestrator): Fix: implement the canvas query view kind, rendering query results as cards on a canvas (reuse the canvas renderer). Acceptance (S5): a query view switched to canvas shows the query's notes as nodes.

### 2026-10-12T15:56:14Z — wil

Status set to todo.

### 2026-10-12T18:28:26Z — /work

Claimed.

### 2026-10-12T18:34:24Z — wil

Verified set to verified.

### 2026-10-12T18:34:24Z — wil

Status set to done.

### 2026-10-12T18:34:24Z — wil

Done on ticket/thevault-prd-42 (bd20830). QueryView kind=canvas now renders CanvasResultsView: results as query-card nodes via CanvasSurface (new optional body prop), each at its view's place block or auto-laid-out (4 per row, 240x176, below the lowest placed card; web/src/views/canvasView.ts). Draws first 200 results with 'Show more' (D33). Verified by e2e query-view.spec.ts 'a query view switched to canvas shows the query's notes as cards...' (fails on old code at the 'not available yet' check, passes now), canvas added to the 50,000-note <300 items test, unit tests canvasView.test.ts, ./dev just check exit 0. Left alone: dragging cards does not save a place — QueryView is rendered read-only (Render passes no onChange), so persisting places would need a write path for the block from view mode; the view pans/zooms unsaved.
