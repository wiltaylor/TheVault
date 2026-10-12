---
id: "13"
decision: D10
---

# Scheduled tasks

## Question
Q13: How do scheduled tasks handle secrets, HTTP and failures?

## Answer
Cron syntax. Secrets live in an encrypted store, referenced by name, never in WCL. HTTP host API restricted to a per-task domain allowlist. Run history with logs. Failures retry with backoff and surface in the UI.

## Why
Agents can edit the vault, so an unrestricted HTTP API would let an agent-written script exfiltrate data.

## Consequences
wscript has no HTTP, so TheVault supplies it as host functions (notes/01-wcl-wscript.md). Secrets are not in files, so they are non-file state like history (Q24).
