---
id: "21"
decision: D19
---

# Offline data scope

## Question
How much is kept offline?

## Answer
Every note on every device. Assets download on open and are cached up to a size limit on Android, with pinned and recent notes' assets prefetched. The desktop keeps all assets.

## Why
Notes are small; assets fill phones; a subset of notes would break offline search and backlinks.

## Consequences
The asset cache limit and eviction policy are implementation details (Accepted unknowns).
