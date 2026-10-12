---
id: '37'
title: Full-text search p95 is 5.0 s at 1M notes, target under 1 s
plan: thevault-prd
blocked_by: []
status: done
verified: unmet
---

# Full-text search p95 is 5.0 s at 1M notes, target under 1 s

Found by ticket 30's S6 bench (./dev just bench, seed 42, 200 samples, reference machine). Full-text search p95 = 4974.8 ms (p50 1868 ms) against the D33 target of under 1 s. 92.6 ms at 20k notes, 436 ms at 100k. The most common vocabulary word takes p95 4948 ms; even rare words take 1.4 s, so the cost looks like per-query work over the whole vault (the hit-to-note join or the page sort) rather than tantivy itself. Reproduce with ./dev just bench.

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Fix: use tantivy top-k collection with cursor paging and resolve only the page's notes; never collect every hit. Acceptance: ./dev just bench shows full-text p95 under 1 s at 1M notes.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T15:56:40Z — /work

Claimed.

### 2026-10-12T17:59:46Z — wil

Verified set to unmet.

### 2026-10-12T17:59:48Z — wil

Status set to done.

### 2026-10-12T17:59:49Z — wil

Fix on ticket/thevault-prd-37 (0f1e323): id is a tantivy fast field; a top-k collector keeps the best page by (score desc, id asc) after the cursor and counts every match, reading no stored docs; query::run pages a plain 'from * search' (what /api/search sends) straight from it and reads only the page's notes from index.db. Old-schema search dirs are recreated on open (every open is followed by a rebuild today: writer, cli local; ticket 38 must keep that in mind). Tests: crates/index/tests/search.rs, query test a_plain_search_pages_in_the_order_the_general_runner_gives; ./dev just check green.
Criterion UNMET (not shown, not disproven): ./dev just bench at 1M, seed 42, 200 samples ran while tickets 36 and 38 ran their own 1M benches and the host was swapping. Full-text search p95 1037.3 ms (was 4974.8), p50 43.1 ms (was 1868). The run is not a clean measurement: open p95 4632.9 ms (was 8.0), save p95 7925.6 ms (was 130.4), reindex 2399.4 s (was 899.5), even though this diff does not touch those paths. Rerun ./dev just bench on an otherwise idle machine to settle it.

### 2026-10-12T18:28:09Z — wil

Merged to main at 67686b9. After merging main (ticket 38's startup now trusts a current index), Index::open_at forgets the generation when Search::open recreates a search dir written with the old schema, so the next open rebuilds; covered by an_index_written_without_the_id_fast_field_is_replaced_and_rebuilt (fails without the change). ./dev just check green on the merged branch. Bench criterion still unmet pending an idle-machine rerun.
