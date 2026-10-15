---
id: '58'
title: Built-in message, call and conversation types; person phones and addresses;
  canvas id suffix
plan: thevault-clients
blocked_by: []
status: done
verified: verified
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

### 2026-10-15T06:06:03Z — /work

Claimed.

### 2026-10-15T06:59:41Z — wil

Verified set to verified.

### 2026-10-15T06:59:43Z — wil

Status set to done.

### 2026-10-15T06:59:44Z — wil

Commit c7108c8 on ticket/thevault-clients-58. S1: crates/server/tests/phone_types.rs (5 tests: new vault, existing vault, undeletable, customisable, conversation query). Vitest: canvas.test.ts freeId tests (2000 ids in one canvas all distinct; 200 fresh ids distinct; forced-repeat suffix retries; existing ids untouched). S2: web/e2e/phone-types.spec.ts. just check green. e2e: two full runs of ./dev just test-e2e at the default 4 workers each had 7-9 failures, a different set each time (kanban paging, pdf, pins, workspace and others), while other fleet workers loaded the machine; every one passed on rerun, and a full run with --workers=2 against the same build passed 250/250 (2 skipped). Assumptions: added a 'hidden' field item for device_contacts; ensure_built_in_types adds a default person type (crates/writer/defaults/person.wcl) when a phone type is added to a vault with no person; an existing person type is not changed; to_raw is a list of strings; no must-keep fields on the new types; suffix is 5 chars [0-9a-z].

### 2026-10-15T07:20:49Z — wil

Rebased onto c2bab8d and fast-forwarded main to 3b1f4d4. On the rebased branch: just check green (546 web tests); just test-e2e 253 passed, 1 failed (columns.spec.ts:239, passed on rerun).
