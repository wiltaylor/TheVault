---
id: '45'
title: 'cli SKILL.md: gaps an agent hit during the agent-only import check'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# cli SKILL.md: gaps an agent hit during the agent-only import check

Found by the ticket 31 agent-only check (D32, D40). The import succeeded, but the agent reported these gaps in crates/cli/SKILL.md: (1) dry-run ids are temporary and differ from the real run's, so references inside one import need several stages; the doc says neither that nor how to reference a note created earlier in the same batch. (2) No JSON example of a content field; the agent fell back to WCL. (3) It does not say the server updates the other side of a two_way reference, nor document the batch report's 'also' field. (4) It does not say whether an upsert key must be unique or whether title may be the key. (5) The batch report has status and error at the top level, which the doc describes only per item; query rows omit content fields so reading a body needs get. Full transcript summary is in ticket 31's worklog.

## Worklog

### 2026-10-12T15:56:14Z — wil

Triage (orchestrator): Fix: update the agent skill document to cover every gap the parity check recorded. Acceptance (S2): the skill-document test runs every shown command, and the gaps listed in this ticket are each addressed.

### 2026-10-12T15:56:14Z — wil

Status set to todo.

### 2026-10-12T18:42:54Z — /work

Claimed.

### 2026-10-12T18:48:54Z — wil

Verified set to verified.

### 2026-10-12T18:48:54Z — wil

Status set to done.

### 2026-10-12T18:48:54Z — wil

SKILL.md now covers all five gaps: staged-import section with a runnable jq example (dry-run ids are placeholders; a batch item cannot refer to a note another item creates, per writer/src/batch.rs), JSON content-block example, two-way references subsection plus 'also', upsert key rules (title or single string/int/url/date/datetime/select; key_not_unique 409 with ids; merge keeps unsent fields), full batch report table incl. top-level status/error, and rows omitting content. New test the_skill_documents_how_batches_report_and_refer in crates/cli/tests/cli.rs asserts each claim. The doc test's blocks now use jq, which the dev container has. just check green.
