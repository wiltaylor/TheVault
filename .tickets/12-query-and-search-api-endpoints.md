---
id: '12'
title: Query and search API endpoints
plan: thevault-prd
blocked_by:
- '11'
- '07'
status: done
verified: verified
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

### 2026-10-12T13:08:55Z — /work

Claimed.

### 2026-10-12T13:12:18Z — wil

Verified set to verified.

### 2026-10-12T13:12:18Z — wil

Status set to done.

### 2026-10-12T13:12:18Z — wil

Endpoints: POST|GET /api/query {query|q, this?, cursor?, page_size? (default 100, max 1000)} -> {rows,next,total,groups}; GET /api/search?q=&cursor=&page_size= (same page shape, runs 'from * search q', best match first); POST /api/query/parse {query} -> {text, model|null, builder_error|null} or 400 query_syntax{line,column,message}; POST /api/query/text {model} -> {text}. Error codes: query_syntax, no_this, bad_cursor, query (unknown type/field/term), invalid_model, bad_request. Writer gained read_index(), which holds the write lock while a query runs. Parse does syntax + builder model only; schema checks (unknown fields) happen when the query runs. S1 tests in crates/server/tests/query.rs.

### 2026-10-12T13:14:02Z — wil

Merged to main at c8645d6. Main had gained ticket 08's auth; POST /api/query, /api/query/parse and /api/query/text were added to READ_POST_PATHS in api/src/auth.rs so a read token can query (otherwise unlisted POSTs need admin).
