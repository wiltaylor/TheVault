---
id: '156'
title: 'AI chat type tools: list, read and propose changes to types and their templates'
plan: ai
blocked_by: []
status: done
verified: verified
---

# AI chat type tools: list, read and propose changes to types and their templates

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md and /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md (this ticket is SPEC-2 D18).

Add list_types, get_type, type_format and propose_type_change to the chat tools in crates/api/src/ai_chat.rs, a dry-run path for type writes if none exists, the type_proposal event, and the panel's type proposal card with a WCL line diff, Apply through the existing type write API with the changed-since check, and Discard. Mention the tools in the shipped chat prompt and docs/ai.md.

Acceptance criteria:
- crates/server tests against FakeAnthropic: a turn that calls list_types, get_type, then propose_type_change with a template edit emits type_proposal with before and after and leaves the type file unchanged; invalid WCL returns an is_error result carrying the validation issues and no proposal; a new type name proposes a new type.
- Web component tests: Apply writes the type, Discard does not, a type changed since the proposal is not applied and says so.
- Extend the ai-chat Playwright e2e: a proposed template change applied from the panel shows on a note of that type.
- ./dev just check passes.

## Worklog

### 2026-10-22T20:57:56Z — wil

Verified set to verified.

### 2026-10-22T20:57:56Z — wil

Status set to done.
