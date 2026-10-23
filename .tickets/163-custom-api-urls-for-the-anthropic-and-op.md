---
id: '163'
title: Custom API URLs for the Anthropic and OpenAI providers
plan: ai
blocked_by: []
status: done
verified: verified
---

# Custom API URLs for the Anthropic and OpenAI providers

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D24.

Expose endpoints.anthropic and endpoints.openai as API URL fields in Settings > AI with the normalising rules, split the ChatGPT plan onto its own endpoint key, make the key optional with a custom URL, add the compatibility errors and the typed model fallback, the host in the trail line, and docs.

Acceptance criteria:
- crates/ai tests: URL normalising (with and without /v1, trailing slashes, non-http schemes and userinfo refused); a ChatGPT plan request goes to the official host even with a custom OpenAI URL set; a custom URL with no key sends no auth header and counts as configured; a 404 on /v1/responses and on /v1/messages gives the named invalid errors.
- crates/server tests: PUT settings stores and clears urls; GET /api/integrations/ai returns them; /api/ai/providers does not; Test works against a FakeOpenAi and a FakeAnthropic on a custom URL with no key.
- Web component tests: the URL field saves and clears; with an empty or failing model list the model becomes a text input.
- ./dev just check passes.

## Worklog

### 2026-10-23T01:11:18Z — wil

Verified set to verified.

### 2026-10-23T01:11:18Z — wil

Status set to done.
