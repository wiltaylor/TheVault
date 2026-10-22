---
id: '154'
title: 'AI chat panel: streamed chat over note tools with edit proposals the user
  applies'
plan: ai
blocked_by:
- '151'
status: done
verified: verified
---

# AI chat panel: streamed chat over note tools with edit proposals the user applies

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md (read it whole; this ticket is its 'Chat panel (ticket A4)' section and D5, D10, D11, D14, D15, D16). Builds on crates/ai from ticket 151. Settings come from ai_settings through crates/ai; ticket 152 builds the page that edits them, so tests write the secrets directly.

Build POST /api/ai/chat (read scope) streaming the listed SSE events with heartbeats, the server-side tool loop (at most 12 rounds per user turn) over search_notes, query_notes, get_note, backlinks and propose_edit (dry-run through Writer::batch, never writes), result truncation, the fixed system prompt file, stopping the loop when the client disconnects, and the web panel: an ai tab in the right sidebar, shortcut and Ask AI command, fetch-stream reader, markdown answer, collapsible tool steps with note links, proposal cards with a diff and Apply (writes through the existing notes API with base_version) and Discard, the refusal notice, Stop, provider and model picker, New conversation, hidden outside the web app. Update crates/cli/skill/thevault/SKILL.md and docs/agents.md briefly, and add the chat section to docs/ai.md (create it if ticket 152 has not landed; merge if it has).

Acceptance criteria:
- crates/server tests against the testkit fakes, for Anthropic and for the ChatGPT plan: a turn that calls search_notes then get_note then answers streams tool, tool_result, text and done events in order, and done.messages replayed as history produces the next request with the earlier tool calls and provider_state intact and unedited; propose_edit emits a proposal and the note is unchanged on disk; a proposal for a wrong base_version reports the conflict to the model; a refusal ends the stream with a refusal event carrying model and category; the 13th tool round is not sent; a client disconnect stops further provider requests.
- Tool results stream only ids and titles in tool_result summaries.
- Web component tests: streamed text renders incrementally, Apply calls the notes write API with base_version, Discard does not, the refusal notice shows the category, Stop aborts.
- One Playwright e2e against a fake provider: ask, see the answer, apply a proposal, see the note change.
- ./dev just check passes.

## Worklog

### 2026-10-22T16:04:18Z — wil

Verified set to verified.

### 2026-10-22T16:04:18Z — wil

Status set to done.
