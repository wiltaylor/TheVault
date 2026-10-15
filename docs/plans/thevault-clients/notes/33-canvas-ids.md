---
id: "33"
decision: D30
---

# Canvas id clashes

## Question
How are canvas ids kept unique across devices?

## Answer
New canvas group, node and edge ids get a short random suffix (for example `text_k3f9`); existing ids are unchanged.

## Why
Readable ids; collisions become negligible.

## Consequences
None.
