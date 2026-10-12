---
id: 09
title: Two-way and peer references and on_delete rules
plan: thevault-prd
blocked_by:
- '07'
status: done
verified: verified
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

### 2026-10-12T13:05:21Z — /work

Claimed.

### 2026-10-12T13:12:49Z — wil

Verified set to verified.

### 2026-10-12T13:12:49Z — wil

Status set to done.

### 2026-10-12T13:12:49Z — wil

Built on ticket/thevault-prd-07 (5619b15) because 07 was done but not merged into main. Step 6 lives in crates/writer/src/refs.rs and runs before Hooks::side_effects. Peer semantics chosen: gaining B completes the group (every member lists every other); losing B takes B out of the whole group. Restrict refusal maps to 409 error=restricted with referrers [{id,type,field}]. Responses gain an also list of other changed notes. Fault injection uses a hidden Writer::set_commit_hook, tested in-process in crates/writer/tests/atomic.rs, because the black-box server has no fault flag. Side-effect updates do not run before_* hooks yet (handlers ticket). Filed a needs-triage ticket for state.db lagging the files after a crash past the commit point.

### 2026-10-12T13:15:14Z — wil

Merged into main at eaed2ff (fast-forward) after merging main in and sending the admin token from the refs tests; ./dev just check green.
