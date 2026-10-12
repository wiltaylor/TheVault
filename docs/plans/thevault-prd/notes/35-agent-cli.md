---
id: "35"
decision: D32
---

# Agent CLI

## Question
Q31: How capable is the CLI?

## Answer
JSON output on every command. Raw WCL read and write. Batch writes from files with `--dry-run`. Upsert by a key field. Asset upload. Query execution. Schema inspection. A bundled agent skill document.

## Why
Agent-driven import (D17) needs batch, upsert and schema discovery.

## Consequences
Batch writes need a defined atomicity rule: whole batch or per item (accepted unknown, see MAP).
