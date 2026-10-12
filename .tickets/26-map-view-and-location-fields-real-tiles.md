---
id: '26'
title: 'Map view and location fields: real tiles and image maps'
plan: thevault-prd
blocked_by:
- '23'
status: todo
verified: unknown
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
