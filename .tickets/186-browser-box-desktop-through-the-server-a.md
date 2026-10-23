---
id: '186'
title: Browser box desktop through the server and its Integrations card
plan: browser
blocked_by: []
status: todo
verified: unknown
---

# Browser box desktop through the server and its Integrations card

Spec: /home/wil/dev/TheVault/.scratch/plans/browser/SPEC.md, B4. Acceptance: server tests with a fake noVNC upstream that /browser/ proxies HTTP and the websocket for a session, refuses tokens of every scope and is absent without VAULT_BROWSER_URL; GET /api/integrations/browser answers reachable and version; a web component test for the card. Targeted tests only.

## Worklog

### 2026-10-23T09:32:38Z — wil

Ticket 185 notes: VAULT_BROWSER_URL is an empty string (not unset) when the box is off - treat empty as unset; /health also answers playwright_cli (the pinned version) beside ok, version, display; /file takes absolute paths or paths relative to /work (playwright-cli prints relative ones); YAML is application/yaml; /files answers {dir, entries:[{name, kind, size, modified_ms}]}.
