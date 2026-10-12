---
id: "08"
decision: D5
---

# Reference behaviour on delete

## Question
Q8: What happens to referencing notes when a target is deleted?

## Answer
Each reference field declares `on_delete`: `unlink` (default), `restrict` or `cascade`.

## Why
SQL foreign-key semantics are well understood.

## Consequences
Deletes route through trash (D13). Unlink fires update events on referrers. Cascade fires delete events on each cascaded note.
