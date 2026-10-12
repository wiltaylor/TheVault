---
id: "15"
decision: D12
---

# Concurrent edits

## Question
Q15: How are concurrent UI and agent edits handled?

## Answer
Optimistic versioning. Every write carries its base version. A stale write returns 409 and the UI shows the conflict.

## Why
Last-write-wins loses data silently. Agents abandon locks.

## Consequences
Out-of-band file edits (Q27) would bypass version checks.
