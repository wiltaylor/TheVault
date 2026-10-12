---
id: '28'
title: 'Canvas: native WCL format, renderer and query nodes'
plan: thevault-prd
blocked_by:
- '23'
- '16'
status: todo
verified: unknown
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
