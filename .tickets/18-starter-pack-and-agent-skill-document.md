---
id: '18'
title: Starter pack and agent skill document
plan: thevault-prd
blocked_by:
- '17'
status: todo
verified: unknown
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
