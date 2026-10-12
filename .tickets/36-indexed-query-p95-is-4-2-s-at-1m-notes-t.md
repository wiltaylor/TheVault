---
id: '36'
title: Indexed query p95 is 4.2 s at 1M notes, target under 500 ms
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# Indexed query p95 is 4.2 s at 1M notes, target under 500 ms

Found by ticket 30's S6 bench (./dev just bench, seed 42, 200 samples, reference machine). Query on indexed fields p95 = 4201.3 ms (p50 1566 ms) against the D33 target of under 500 ms. Time grows roughly linearly with vault size: 72.8 ms at 20k notes, 369 ms at 100k. Slowest shapes at 1M: 'from task where status == "Active/In progress" sort due asc' p95 4708 ms, 'from * sort updated desc' p95 3705 ms. Even a selective 'from task where project == <id>' takes 1.4 s, which points at a scan rather than an index lookup. Reproduce with ./dev just bench (or --notes 100000 for a faster signal).

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Fix: push where/sort/limit/cursor into SQL with indexes on (type, field, value) and on reference and taxonomy tables, and stop materialising whole result sets in Rust. Acceptance: ./dev just bench shows indexed query p95 under 500 ms at 1M notes, and S4 query tests still pass.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T15:56:35Z — /work

Claimed.

### 2026-10-12T19:02:41Z — wil

Verified set to verified.

### 2026-10-12T19:02:41Z — wil

Commit 1115e3f on ticket/thevault-prd-36. ./dev just bench (1M notes, seed 42, 200 samples) reports 'Query on indexed fields, p95 | under 500 ms | 271.9 ms | p50 114.9 ms | max 545.7 ms | pass' (was 4201.3 ms). Per shape p95: status== sort due 131 ms; under(status)&&due< 280 ms; priority&&!done sort updated desc 197 ms; project== 6 ms; owner== 6 ms; has(topics) 54 ms; under(topics)&&rating 124 ms; from * sort updated desc 54 ms; estimate>8&&!done group by status 236 ms. S4: cargo test -p query 21 passed (the original 17 plus differential tests that compare the SQL plan with the note-by-note scan page by page). ./dev just check exit 0. Design: query/src/plan.rs translates filter/sort/group exactly when it can (else falls back to the old scan, unchanged); one top-level field condition drives from field_values_by_str/num (now (field, type, value)), the rest are PK probes; one statement returns the order keys of all matches and Rust cuts total/groups/page from them, reading only the page's notes; sort updated/created desc or unsorted walks the notes index when the match count says it reads less. Not done: links/terms indexes (linked_from and dotted paths still run note by note), search queries (ticket 37). Cost: index.db v2 is ~20% larger and vault reindex uses ~9% more CPU at 100k (user+sys 11.65 s vs 10.7 s, three alternating runs each). The same 1M run measured reindex 2124.8 s (FAIL vs 30 min) and server start 1842.8 s, against 899.5 s / 1094.5 s in ticket 30's run; the machine had load 11-15 from other workers' 1M benches, so most of that is contention, but reindex at 1M should be re-measured on a quiet machine.

### 2026-10-12T19:02:41Z — wil

Status set to done.

### 2026-10-12T19:13:57Z — wil

Merged into main as c4362f3 (fast-forward) after merging main into the branch: resolved conflicts in query/src/run.rs (kept ticket 37's plain_search fast path ahead of the planner) and query/tests/query.rs (kept both sides' tests); bumped index.db VERSION to 3 because main's ticket 38 also used 2. ./dev just check exit 0 on the merged tree.
