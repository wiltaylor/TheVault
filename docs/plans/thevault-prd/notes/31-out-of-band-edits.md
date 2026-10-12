---
id: "31"
decision: D28
---

# Direct file edits

## Question
Q27: Are direct edits to files on disk supported?

## Answer
No. All edits go through the API. `vault reindex` validates files and reports problems after a manual edit.

## Why
A watcher must handle partial writes, swap files and unintended handler runs, and bypasses version checks.

## Consequences
Manual edits skip handlers and history. The PRD states this.
