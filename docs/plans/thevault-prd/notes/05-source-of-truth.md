---
id: "05"
decision: D2
---

# Source of truth

## Question
Q2: WCL files or database as source of truth?

## Answer
WCL files on disk are the source of truth. A SQLite index is derived from them and can be rebuilt.

## Why
Matches the brief that WCL is the storage format. ai-kg proves the pattern (notes/02-prior-art.md).

## Consequences
Unblocks dialect (Q3), formatting (Q4) and file layout (Q5). Raises whether out-of-band file edits are supported (Q27). History (D13) lives outside the files, so the index is not the only non-file state (Q24).
