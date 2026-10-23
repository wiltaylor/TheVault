---
id: '165'
title: Brave Search integration and the AI web_search tool
plan: ai
blocked_by: []
status: todo
verified: unknown
---

# Brave Search integration and the AI web_search tool

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D26.

Add the brave_api_key secret, the Brave Search card in Settings > Integrations with key, Test and Remove, the /api/integrations/brave routes, the web_search chat tool offered only with a key, the panel step, FakeBrave, and docs.

Acceptance criteria:
- crates/server tests against FakeBrave: the key stores, never comes back, and deletes; Test reports the count or Brave's error; with no key the chat tool list has no web_search; with a key a turn calling web_search gets stripped titles, urls and descriptions; 429 and 401 return is_error results with the right kind; the query never appears in the trail line.
- Web component tests: the Brave card's key, Test and Remove; the web_search step shows the results as outbound links.
- ./dev env CI=true just check passes.

## Worklog
