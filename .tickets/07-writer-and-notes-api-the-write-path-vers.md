---
id: '07'
title: 'writer and notes API: the write path, versions and 409s'
plan: thevault-prd
blocked_by:
- '04'
- '05'
- '06'
status: todo
verified: unknown
---

# writer and notes API: the write path, versions and 409s

Implement the `writer` crate and the first `api` endpoints for notes.

Scope:
- The SPEC.md write path steps 1 to 4 and 7 to 11 (handlers and reference side effects are later tickets, so leave clean extension points for steps 5 and 6 and the after-queue).
- Notes API: create, read, update, delete (to trash), list by type, history, restore version, trash list and restore. Accept and return WCL text or JSON (D32).
- Every write carries a base version. Stale writes return 409 with the current version (D12).
- Every write response returns canonical WCL and the new version (D21).
- Authentication is stubbed behind a single extractor that ticket `auth` replaces; tests use it.

Acceptance criteria (S1):
- Create, read, update and delete a note over HTTP; the file appears, changes and moves to trash as expected (S3).
- An update with a stale base version returns 409 and leaves the file unchanged.
- A note with an expression returns 400 naming the item.
- History lists every version with author; restoring an old version creates a new version with the old content.
- The write response body equals the file on disk.

Seam: S1 with S3 checks. Decisions D2, D12, D13, D20, D21, D32.

## Worklog
