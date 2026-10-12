---
id: '18'
title: Starter pack and agent skill document
plan: thevault-prd
blocked_by:
- '17'
status: done
verified: verified
---

# Starter pack and agent skill document

Ship `vault init --starter` content and the agent skill document (D32, D37).

Scope:
- Starter: Task, Project, Person and Note types, a Status taxonomy, and a dashboard note with kanban and calendar query views.
- Agent skill document (a SKILL.md-style file shipped with the binary, printed by `vault skill`) teaching an agent to inspect schemas, dry-run and run batches, upsert, query, and fix validation errors.

Acceptance criteria (S2):
- `vault init --starter` creates the starter files and they all validate; `vault reindex` reports no errors.
- The dashboard note's queries run and return starter notes.
- `vault skill` prints the document; every command it shows runs successfully against a starter vault in a test.

Seam: S2. Decisions D32, D37.

## Worklog

### 2026-10-12T14:04:05Z — /work

Claimed.

### 2026-10-12T14:12:05Z — wil

Verified set to verified.

### 2026-10-12T14:12:05Z — wil

Status set to done.

### 2026-10-12T14:12:05Z — wil

Branch ticket/thevault-prd-18 @ e2a8c4a. Merged ticket/thevault-prd-17 in first, because 17 (the CLI) was done but not yet on main; merge 17 before or with 18. Starter files live in crates/cli/starter/ and are written by crates/cli/src/starter.rs (definitions straight to the store, notes through writer, author session 'vault'). Due dates are relative to the UTC day of init. The skill document is crates/cli/SKILL.md; every ```sh block in it is run by the S2 test every_command_in_the_skill_document_runs_against_a_starter_vault with $ID/$VERSION set to a fresh task, so any command added to the doc must go in an sh block and must succeed. Starter fields with defaults are optional because creates do not apply defaults yet (ticket 33); the doc tells agents to set fields explicitly. Seen in passing: a client write with a field the type does not define is accepted and flagged rather than refused.
