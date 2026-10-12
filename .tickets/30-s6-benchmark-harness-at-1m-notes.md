---
id: '30'
title: S6 benchmark harness at 1M notes
plan: thevault-prd
blocked_by:
- '17'
- '12'
status: done
verified: unmet
---

# S6 benchmark harness at 1M notes

Build the S6 benchmark (D31, D33, D39).

Scope:
- A generator that creates a 1M-note vault (mixed types, references, taxonomies, content) through the batch API or directly through `store` for speed (document which).
- Timed runs: indexed query p95, full-text p95, save p95 excluding handlers, note open p95, full reindex time.
- `just bench` runs it and writes a report comparing against the targets in PRD Success criteria.

Acceptance criteria:
- `./dev just bench` produces a report with every measure and pass/fail against its target on the reference machine.
- The generator is deterministic with a seed.
- Any target missed is listed in the ticket worklog with the measured value; missing a target is `unmet`, not hidden.

Seam: S6. Decisions D31, D33, D39.

## Worklog

### 2026-10-12T14:21:38Z — /work

Claimed.

### 2026-10-12T15:45:27Z — wil

1M run on the reference machine (Ryzen 9 9950X, 186 GiB, btrfs NVMe), ./dev just bench, seed 42, 200 samples per measure, commit a5cd066. MISSED: query on indexed fields p95 4201.3 ms (target under 500 ms) -> ticket 36. MISSED: full-text search p95 4974.8 ms (target under 1 s) -> ticket 37. Met: save p95 130.4 ms (under 200 ms), open p95 8.0 ms (under 300 ms), vault reindex 899.5 s (under 30 min). Also: generation 39 s through store, 2.0 GiB of note files; vault-server start 1094.5 s because Writer::open rebuilds the index -> ticket 38. Earlier scaling runs: 20k notes query p95 72.8 ms, search 92.6 ms; 100k notes query 369 ms, search 436 ms, reindex 22 s. Save at 20k/100k was 650-820 ms but 130 ms at 1M, probably from the container's first-run warm-up. Not explained, not hidden. Full report: target/bench/report.md in the worktree.

### 2026-10-12T15:45:27Z — wil

Verified set to unmet.

### 2026-10-12T15:45:27Z — wil

Status set to done.

### 2026-10-12T15:46:54Z — wil

Merged into main as bc2c5e6 (fast-forward), after merging main into the branch with no conflicts and passing ./dev just check (exit 0). The 1M figures were measured at a5cd066, before main gained 695d39d (state.db catch-up on writer open), which may change server start time.
