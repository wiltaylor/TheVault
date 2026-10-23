---
id: '172'
title: 'Agent threads and agent turns: stored threads, the agent''s permissions applied,
  the Chat tab and the AI panel''s agent picker'
plan: agents
blocked_by:
- '170'
status: done
verified: verified
---

# Agent threads and agent turns: stored threads, the agent's permissions applied, the Chat tab and the AI panel's agent picker

Spec: /home/wil/dev/TheVault/.scratch/plans/agents/SPEC.md, G4, G5 Chat tab, G6, G7, G8, and G11's Author::Agent for applied changes. Acceptance: server tests that a turn with agent and thread uses the agent's model, prompt and tools, refuses notes outside permissions.types in every note tool, applies its own confirm switches, is capped by the caller's scope, stores messages and proposal outcomes as it streams, and reloads the thread's history itself; thread routes (list, create, get, rename, delete); web tests for the Chat tab (threads, new thread, pending badge, reopened pending cards) and the panel's agent picker; the app turns form with agent and thread. Targeted tests only.

## Worklog

### 2026-10-23T03:31:51Z — wil

Design change before work started (user, 2026-10-23): threads are notes of a new built-in type agent_thread (SPEC G7 rewritten), proposal state lives in the thread note's transcript (G8). Acceptance changes: replace the state.db thread table tests with tests that a thread note is created, its transcript and body are written as the turn streams, it is authored Author::Agent, note tools leave agent_thread out unless permissions.types names it, and a fresh vault and an upgraded vault both get the agent_thread type.

### 2026-10-23T03:32:14Z — wil

Design change (user, 2026-10-23, SPEC G6): every AI panel conversation is a thread of the chosen agent, Assistant by default; the chat reads system prompt, confirm and web from the agent, no longer from ai_settings; an upgrade step moves ai_settings.system_prompt, confirm and web into agents/assistant.wcl and clears them. Add tests for the migration and that /api/ai/providers still answers confirm and web from Assistant.

### 2026-10-23T04:29:32Z — wil

Verified set to verified.

### 2026-10-23T04:29:32Z — wil

Status set to done.
