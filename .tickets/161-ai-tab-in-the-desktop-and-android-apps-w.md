---
id: '161'
title: AI tab in the desktop and Android apps, with chat forwarded to the home server
plan: ai
blocked_by:
- '160'
status: todo
verified: unknown
---

# AI tab in the desktop and Android apps, with chat forwarded to the home server

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D23 (and D14, D16, D22 it amends).

Add GET /api/ai/providers (read), the turns form of the chat (POST /api/ai/chat/turns, GET .../{turn}/events, DELETE .../{turn}) sharing one runner with POST /api/ai/chat, the embedded server's forwarding of the AI routes with the device token, the panel in app mode using the turns form and /api/ai/providers, local apply with the sync-and-retry rule, and the app-mode messages.

Acceptance criteria:
- crates/server tests: a turn started with POST turns streams the same events as POST /api/ai/chat; a turn with no reader for 30 s stops and makes no further provider requests; DELETE stops it; a fifth concurrent turn for one caller is refused; GET /api/ai/providers works with a read token and carries no key, token or account.
- crates/embedded tests: the AI routes are forwarded to a test home server with the device token and the events stream through; an unreachable server answers ai_needs_server.
- Web component tests: in app mode the AI tab shows, uses the turns form, shows Set up AI in the web app when nothing is configured, and retries Apply once after a sync when the version differs.
- ./dev just check and ./dev just android-test pass.

## Worklog
