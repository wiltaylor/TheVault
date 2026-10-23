---
id: '173'
title: 'Agent triggers: schedules and note events, unattended runs with pending changes,
  limits and no loops'
plan: agents
blocked_by:
- '172'
status: todo
verified: unknown
---

# Agent triggers: schedules and note events, unattended runs with pending changes, limits and no loops

Spec: /home/wil/dev/TheVault/.scratch/plans/agents/SPEC.md, G9 to G12 and the Triggers tab of G5. Acceptance: scheduler tests that a schedule trigger starts a thread with its prompt; writer or server tests that create, update and delete events with type and where filters start runs with the event line, debounce within 30 s, never fire for Author::Agent writes, never fire on the embedded servers; unattended proposals stay pending with one bell notification per run; confirm off applies at once; 2 concurrent and 60 per hour limits; agent_runs recorded and shown in the Triggers tab (web test). Targeted tests only.

## Worklog
