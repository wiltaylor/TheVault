---
id: '58'
title: Built-in message, call and conversation types; person phones and addresses;
  canvas id suffix
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
---

# Built-in message, call and conversation types; person phones and addresses; canvas id suffix

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC built-in types (D15, D24, D30): `message`, `call`, `conversation` as built-in types (customisable, not deletable) through the existing built-in type list, with the fields exactly as SPEC lists, `conversation`'s default template showing a query view of its messages; `person` starter type gains `phones` (list of strings), `addresses` (list of strings) and hidden `device_contacts` dict; existing vaults get the built-in types ensured on start. New canvas group/node/edge ids get a short random suffix (web/src/canvas freeId) without changing existing ids. Document in docs/vault-format.md.

Acceptance criteria:
- S1: a new and an existing vault both have the three types; they cannot be deleted; can be customised.
- Vitest: freeId produces suffixed ids that do not collide across many generations; existing ids untouched.
- S2: a conversation note shows its messages in the query view.
- `./dev just check` and `./dev just test-e2e` pass.
Decisions: D15, D24, D30.

## Worklog
