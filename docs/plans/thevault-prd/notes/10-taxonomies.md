---
id: "10"
decision: D7
---

# Taxonomies

## Question
Q10: Shape of taxonomies?

## Answer
Global, hierarchical taxonomies. Each field declares single or multi-valued. Kanban requires a single-valued field.

## Why
Status and topic taxonomies are reused across types. Hierarchy is cheap now and painful later.

## Consequences
Querying must support hierarchy (match a term and its descendants).
