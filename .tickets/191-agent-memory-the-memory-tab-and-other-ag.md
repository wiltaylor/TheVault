---
id: '191'
title: 'Agent memory: the Memory tab and other agents'' memory tools'
plan: agents
blocked_by:
- '190'
status: todo
verified: unknown
---

# Agent memory: the Memory tab and other agents' memory tools

Spec: /home/wil/dev/TheVault/.scratch/plans/agents/SPEC.md section G21 (Memory tab, cross-agent tools with confirm.agents and the escalation rule, docs, and their tests). Targeted tests only (no release yet).

## Worklog

### 2026-10-24T02:32:24Z — wil

Ticket 190 notes: helpers in crates/api/src/agent_memory.rs (memories, matching, read, create, update, forget, prompt_section; Memory {id, agent, title, text, version, created, updated}; limits and error codes invalid_memory/memory_full/not_found); writer::AGENT_MEMORY_TYPE; own tools dispatch at the top of ai_chat::run_tool via is_memory_tool(), so the cross-agent tool names must not match it; no memory HTTP routes yet (add them for the Memory tab on the helpers, read for GET, write for changes).
