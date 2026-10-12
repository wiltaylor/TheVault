---
id: "32"
decision: D29
---

# Frontend libraries and tiles

## Question
Q28: Which libraries are constraints?

## Answer
TipTap/ProseMirror for the editor, OpenLayers for maps, perfect-freehand for ink, CodeMirror 6 for code, FullCalendar core (MIT) for calendar, vis-timeline for timeline. Tiles default to OpenFreeMap with a configurable tile URL. Written in the PRD as constraints with reasons.

## Why
Rules out licence traps: tldraw production licence, BlockNote GPL packages, FullCalendar paid timeline (notes/03-frontend-libs.md).

## Consequences
OpenStreetMap attribution is required in the map UI.
