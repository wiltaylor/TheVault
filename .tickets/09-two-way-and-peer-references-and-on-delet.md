---
id: 09
title: Two-way and peer references and on_delete rules
plan: thevault-prd
blocked_by:
- '07'
status: todo
verified: unknown
---

# Two-way and peer references and on_delete rules

Implement step 6 of the write path: reference side effects.

Scope:
- Two-way relationships: setting a reference updates the paired field on the other note in the same atomic commit; single and list cardinalities (D4).
- Peer relationships: adding B to A's list adds A to B's list and to every member as the format defines (D4).
- Reverse-side updates bump the other note's version, so a stale copy of it gets 409 (SPEC write path).
- `on_delete`: `unlink`, `restrict` (fail and list referencing notes), `cascade` (D5).

Acceptance criteria (S1 + S3):
- Linking a task to a project writes the link into both files in one commit.
- Removing the link removes it from both files.
- A peer add appears on every member's file.
- Writing the other note with its pre-link version returns 409.
- Deleting a target with `unlink` removes the reference from referrers; with `restrict` returns 409 or 422 listing referrers and changes nothing; with `cascade` trashes the referrers.
- A fault injected mid-commit leaves neither side changed.

Seam: S1 and S3. Decisions D4, D5, D12.

## Worklog
