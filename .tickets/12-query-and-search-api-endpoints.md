---
id: '12'
title: Query and search API endpoints
plan: thevault-prd
blocked_by:
- '11'
- '07'
status: todo
verified: unknown
---

# Query and search API endpoints

Expose queries and search over HTTP.

Scope:
- Run a query with an optional `this` note id; cursor pagination in the response.
- Full-text search endpoint.
- Parse/validate endpoint returning the builder model or errors, for the UI builder.

Acceptance criteria (S1):
- A query over notes created through the API returns them, paged with a cursor.
- A query with a syntax error returns 400 with line and column.
- A query using `this` without a note id returns 400.
- Search finds a note by a word in its content.

Seam: S1. Decisions D8, D16, D33.

## Worklog
