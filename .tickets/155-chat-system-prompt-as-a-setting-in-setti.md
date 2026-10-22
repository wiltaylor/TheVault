---
id: '155'
title: Chat system prompt as a setting in Settings > AI
plan: ai
blocked_by: []
status: done
verified: verified
---

# Chat system prompt as a setting in Settings > AI

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md and /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md (this ticket is SPEC-2 D17).

Add ai_settings.system_prompt, expose it and the default through GET /api/integrations/ai, accept it in PUT .../settings (empty resets), use it in POST /api/ai/chat for every provider with the note context appended after it, and add the Chat system prompt card to web/src/pages/Ai.tsx. Document in docs/ai.md.

Acceptance criteria:
- crates/server test: with no setting the chat request's system text is ai::CHAT_PROMPT; after PUT it is the custom text followed by the note context; an empty PUT resets; over 32 000 characters is a 400.
- Web component test: the card shows the prompt in use, Save sends it, Reset to default restores and is disabled when not customised.
- ./dev just check passes.

## Worklog

### 2026-10-22T20:39:12Z — wil

Verified set to verified.

### 2026-10-22T20:39:12Z — wil

Status set to done.
