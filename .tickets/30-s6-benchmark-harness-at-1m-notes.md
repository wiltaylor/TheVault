---
id: '30'
title: S6 benchmark harness at 1M notes
plan: thevault-prd
blocked_by:
- '17'
- '12'
status: todo
verified: unknown
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
