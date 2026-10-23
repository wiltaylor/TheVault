---
id: '171'
title: 'Agents in the left sidebar and the agent screen: Settings, Permissions and
  File tabs'
plan: agents
blocked_by:
- '170'
status: todo
verified: unknown
---

# Agents in the left sidebar and the agent screen: Settings, Permissions and File tabs

Spec: /home/wil/dev/TheVault/.scratch/plans/agents/SPEC.md, G3 and G5 (the Settings, Permissions and File tabs; the Chat and Triggers tabs come with tickets 3 and 4). Acceptance: web component tests for the sidebar section (list, +, open, disable, delete with confirm), the Settings tab (model list, system prompt, one Save), the Permissions tab (tools, switches, types) and the File tab; read-only in app mode; a Playwright e2e that creates an agent from the sidebar, sets its model and prompt, and sees it listed. Targeted tests only.

## Worklog

### 2026-10-23T03:32:14Z — wil

Design change (user, 2026-10-23, SPEC G6): Assistant is listed first in the sidebar with no Delete or Disable; Settings > AI loses the Chat system prompt and Changes the AI makes cards in favour of a link to Assistant's agent screen, whose Settings and Permissions tabs edit the same things. Add tests for both.
