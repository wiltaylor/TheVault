---
id: "30"
decision: D27
---

# Query syntax

## Question
Q26: What is the query language based on?

## Answer
A purpose-built query language whose expressions use wscript syntax, for example `from task where status = "done" and due < today() sort due desc`.

## Why
One expression grammar across queries and scripts. Easier for the visual builder to round-trip than SQL.

## Consequences
The grammar must cover hierarchy matching for taxonomies (D7), traversal of references, full-text match (D16), sort, group (kanban) and limit.
