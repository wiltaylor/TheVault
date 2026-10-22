---
id: '161'
title: AI tab in the desktop and Android apps, with chat forwarded to the home server
plan: ai
blocked_by:
- '160'
status: done
verified: verified
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

### 2026-10-22T21:24:36Z — wil

Design change before work started (user, 2026-10-23): every AI change is applied on the home server through a new POST /api/ai/apply (write scope for note ops, admin for type ops, 403 needs_admin otherwise), shared with D22's auto-apply; the web panel's Apply and Apply all switch to it; the embedded server forwards it and never applies AI changes locally; after an apply or an applied event the app requests a sync round and refreshes the open view. This replaces the local-apply-with-sync-retry rule; SPEC-2 D23 is updated. Extra acceptance criteria: crates/server tests that /api/ai/apply applies each op with the version check, refuses a type op without admin with needs_admin, and refuses a stale base_version; a crates/embedded test that /api/ai/apply is forwarded and the local vault is untouched until sync; a web test that the app requests a sync after Apply.

### 2026-10-22T22:23:05Z — wil

Verified set to verified.

### 2026-10-22T22:23:05Z — wil

Status set to done.
