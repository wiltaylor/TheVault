---
id: "36"
decision: D33
---

# Performance targets at 1M notes

## Question
Q33: What are the performance targets at 1M notes?

## Answer
At 1M notes: indexed-field query p95 under 500 ms; full-text search p95 under 1 s; save excluding handlers p95 under 200 ms; note open p95 under 300 ms; full `vault reindex` under 30 minutes. Every view (table, list, cards, kanban columns, calendar, timeline, map pins, canvas query nodes) paginates or virtualises and never loads a whole result set.

## Why
Testable and reachable with SQLite plus tantivy. Unpaginated views fail in the browser before the backend does.

## Consequences
The query language and API need cursor pagination. Reference and taxonomy aggregation in views must be computed server-side.
