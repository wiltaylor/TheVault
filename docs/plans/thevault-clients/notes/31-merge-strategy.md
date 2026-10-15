---
id: "31"
decision: D28
---

# Merge strategy

## Question
How are blocks merged without block ids?

## Answer
Positional three-way merge: each top-level block is hashed from its canonical text, and base, device and server block lists merge diff3-style. Ambiguous alignment falls back to a conflict copy. Fields merge individually. No format change.

## Why
No format or editor change; option of per-field clashes would make most offline edits conflicts (note 26).

## Consequences
Reordered blocks produce conflict copies more often; acceptable.
