---
id: '172'
title: 'Agent threads and agent turns: stored threads, the agent''s permissions applied,
  the Chat tab and the AI panel''s agent picker'
plan: agents
blocked_by:
- '170'
status: todo
verified: unknown
---

# Agent threads and agent turns: stored threads, the agent's permissions applied, the Chat tab and the AI panel's agent picker

Spec: /home/wil/dev/TheVault/.scratch/plans/agents/SPEC.md, G4, G5 Chat tab, G6, G7, G8, and G11's Author::Agent for applied changes. Acceptance: server tests that a turn with agent and thread uses the agent's model, prompt and tools, refuses notes outside permissions.types in every note tool, applies its own confirm switches, is capped by the caller's scope, stores messages and proposal outcomes as it streams, and reloads the thread's history itself; thread routes (list, create, get, rename, delete); web tests for the Chat tab (threads, new thread, pending badge, reopened pending cards) and the panel's agent picker; the app turns form with agent and thread. Targeted tests only.

## Worklog
