---
id: '170'
title: 'Agent files: agents/<name>.wcl vocabulary, validation, writer and /api/agents
  routes'
plan: agents
blocked_by: []
status: todo
verified: unknown
---

# Agent files: agents/<name>.wcl vocabulary, validation, writer and /api/agents routes

Spec: /home/wil/dev/TheVault/.scratch/plans/agents/SPEC.md, G1, G2 and the G4 permission fields (enforcement is ticket 3). Acceptance: schema tests for every field and refusal (bad name, unknown provider, bad cron, unknown trigger op, unknown type in permissions.types or a trigger, bad where query); writer tests for create, update, delete into history; server tests for the routes and their scopes (read lists, write is refused on PUT and DELETE, admin writes); docs/vault-format.md Agent files. Targeted tests only (the full gate runs before the next release).

## Worklog

### 2026-10-23T03:32:14Z — wil

Design change (user, 2026-10-23): ship a built-in agents/assistant.wcl (SPEC G6): no provider, model or system_prompt, all tools and types, no triggers; DELETE and enabled=false are refused with 400 built_in. Add tests for both refusals and for a fresh vault and an upgraded vault getting the file.
