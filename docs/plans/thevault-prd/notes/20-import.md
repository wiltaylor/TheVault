---
id: "20"
decision: D17
---

# Importers

## Question
Q20: Are importers in scope?

## Answer
No built-in importers. AI agents import data from other tools through the API and CLI.

## Why
User decision: agent-driven import replaces importer code.

## Consequences
The API and CLI must make bulk creation practical: batch writes, asset upload, and idempotent upserts (Q31).
