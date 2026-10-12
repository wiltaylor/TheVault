---
id: '44'
title: 'web: dragging a map pin on a stale version reverts with no conflict view'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# web: dragging a map pin on a stale version reverts with no conflict view

Found by the ticket 31 parity audit. web/src/map/MapView.tsx:66-72 re-reads the note then PUTs it; any error, including a 409, just reverts the pin. SPEC 'Web client' says a 409 from the API opens a conflict view, as NoteView and KanbanView do. Reported by code reading, not reproduced in a browser.

## Worklog

### 2026-10-12T15:56:14Z — wil

Triage (orchestrator): Fix: a 409 on pin drag opens the conflict view like kanban does. Acceptance (S5): a stale pin drag shows the conflict view and loses no data.

### 2026-10-12T15:56:14Z — wil

Status set to todo.

### 2026-10-12T18:36:44Z — /work

Claimed.

### 2026-10-12T18:50:32Z — wil

Verified set to verified.

### 2026-10-12T18:50:32Z — wil

Status set to done.

### 2026-10-12T18:50:32Z — wil

Fixed on ticket/thevault-prd-44. Root cause was worse than reported: MapView read the note at drop time, so a concurrent save was silently overwritten rather than 409ing. Now reads at drag start (new onDragStart hook in ol.ts/MapCanvas), PUTs against that version, opens ConflictView on 409; Keep stored resets the pin to the stored point. e2e: map.spec.ts 'a pin dragged over a stale version...' and 'keeping the stored version...'. Full e2e run under load avg ~21 had 31 timeout failures in unrelated specs; all passed on rerun with fewer workers.

### 2026-10-12T19:02:33Z — wil

Merged into main at 0a14764 (fast-forward after merging main into the branch; just check green).
