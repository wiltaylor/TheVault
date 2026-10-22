---
id: '160'
title: Settings > AI switches to turn off confirmation per kind of AI change
plan: ai
blocked_by:
- '158'
status: done
verified: verified
---

# Settings > AI switches to turn off confirmation per kind of AI change

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md section D22 (D14, D18, D20 for the proposal tools it switches).

Add ai_settings.confirm, its GET and PUT handling, the apply-in-the-tool path with the applied event and the scope fallback, the Changes the AI makes card in web/src/pages/Ai.tsx, the panel's applied card with view refresh, and docs/ai.md.

Acceptance criteria:
- crates/server tests against FakeAnthropic: with defaults every tool proposes and writes nothing; with notes off, propose_edit and propose_create write as the caller and emit applied, and the tool result carries the new version; a version conflict writes nothing and returns is_error; with delete off the note goes to the trash and is never purged; with types off a template change is applied through the planned change logic; a read-scope token with every switch off still gets proposals.
- Web component tests: the four switches show on by default, toggling one saves a partial confirm; the applied card renders for each op and links correctly.
- Extend the ai-chat Playwright e2e: with notes confirmation off, an edit asked for in the panel shows in the open note without pressing Apply.
- ./dev just check passes.

## Worklog

### 2026-10-22T21:46:58Z — wil

Verified set to verified.

### 2026-10-22T21:46:58Z — wil

Status set to done.
