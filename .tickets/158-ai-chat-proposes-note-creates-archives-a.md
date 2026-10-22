---
id: '158'
title: AI chat proposes note creates, archives and deletes as well as updates
plan: ai
blocked_by:
- '156'
status: todo
verified: unknown
---

# AI chat proposes note creates, archives and deletes as well as updates

Spec: /home/wil/dev/TheVault/.scratch/plans/ai/SPEC.md and /home/wil/dev/TheVault/.scratch/plans/ai/SPEC-2.md (this ticket is SPEC-2 D20).

Add propose_create, propose_archive and propose_delete to the chat tools beside propose_edit, the op field on proposals, the panel's cards for each with Apply through the existing notes routes and the changed-since checks, Apply all, and the docs and chat prompt entries.

Acceptance criteria:
- crates/server tests against FakeAnthropic: each tool emits a proposal with the right op and writes nothing; an invalid create returns an is_error result with the validation issues; archiving an already archived note and deleting a missing note return is_error results.
- Web component tests: Apply for create, archive, unarchive and delete each calls the right route; a note changed since a delete proposal is not deleted; Apply all applies in order and stops at the first failure.
- Extend the ai-chat Playwright e2e: a proposed create applied from the panel opens the new note, and a proposed delete applied puts the note in the trash.
- ./dev just check passes.

## Worklog
