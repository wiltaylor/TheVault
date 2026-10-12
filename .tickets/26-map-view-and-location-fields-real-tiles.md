---
id: '26'
title: 'Map view and location fields: real tiles and image maps'
plan: thevault-prd
blocked_by:
- '23'
status: done
verified: verified
---

# Map view and location fields: real tiles and image maps

Build maps on OpenLayers (D29).

Scope:
- Map view over location fields: geo maps on OpenFreeMap tiles by default with a configurable tile URL and OpenStreetMap attribution.
- Image maps: an uploaded image as the map with a pixel coordinate system; pins placed and moved on it.
- Location field editor: pick on a map (geo or a chosen image map).

Acceptance criteria (S5):
- Notes with geo locations render as pins at the right coordinates (Playwright uses a local tile stand-in so tests do not hit the internet).
- Changing the tile URL setting changes the tile requests.
- An image map shows the image and pins placed on it save pixel coordinates and reload at the same spot.
- Attribution is visible on geo maps.

Seam: S5. Decisions D3, D29.

## Worklog

### 2026-10-12T14:07:20Z — /work

Claimed.

### 2026-10-12T14:20:22Z — wil

Verified set to verified.

### 2026-10-12T14:20:22Z — wil

Status set to done.

### 2026-10-12T14:20:22Z — wil

Committed f7284c0 on ticket/thevault-prd-26. Assumption: the tile URL setting is per-browser (localStorage, key thevault.map.tiles) and edited on a new /settings page, because no server/vault settings mechanism exists in SPEC or vault-format.md. It accepts a MapLibre style URL (default https://tiles.openfreemap.org/styles/liberty, via ol-mapbox-style) or a raster {z}/{x}/{y} template, since OpenFreeMap serves vector styles only. Pins are DOM overlays paged 500 at a time (D33); dragging a pin in the map view writes the note. web/e2e/fixtures.ts now aborts every request to a non-server origin unless a test routes it, so the geo map in the every-field-kind test no longer reaches the internet. Evidence: map.spec.ts 4/4, full e2e 37/37, ./dev just check exit 0. A mutation (dropping the image y-flip) failed both image tests. Merge note: ticket 25 also edits views/QueryView.tsx Switch and views/model.ts ViewDef; expect small textual conflicts there.

### 2026-10-12T14:27:09Z — wil

Merged into main at d1de9e9 (fast-forward). Merging main brought in tickets 18, 24 and 25. The conflicts in QueryView.tsx, views/model.ts, QueryViewSettings.tsx, styles.css and pnpm-lock.yaml were all resolved by keeping both sides; the lockfile was regenerated. The first CSS resolution left .settings-actions unclosed, which swallowed .kanban and failed date-views.spec.ts:59. Fixed in d1de9e9. After the merge: ./dev just check exit 0, full e2e 46/46.
