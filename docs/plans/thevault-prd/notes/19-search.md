---
id: "19"
decision: D16
---

# Full-text search

## Question
Q19: Full-text search in the release?

## Answer
Yes. Full-text search over fields and content blocks, available from the query language.

## Why
A note app without search does not scale past a few hundred notes.

## Consequences
The index is rebuildable from files, so the search index is too.
