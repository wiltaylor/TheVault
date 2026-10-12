---
id: '07'
title: 'writer and notes API: the write path, versions and 409s'
plan: thevault-prd
blocked_by:
- '04'
- '05'
- '06'
status: done
verified: verified
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

### 2026-10-12T12:55:10Z — /work

Claimed.

### 2026-10-12T13:04:51Z — wil

Verified set to verified.

### 2026-10-12T13:04:51Z — wil

Status set to done.

### 2026-10-12T13:04:51Z — wil

Branch ticket/thevault-prd-07 @ 5619b15, based on ticket/thevault-prd-06 (fc672e3), which was done but not yet merged to main: merge 06 first. Criteria proven by crates/server/tests/notes.rs (8 S1 tests with S3 file checks); mutation-checked: disabling the base-version comparison fails the 409 and restore tests. Design: writer::Writer::apply is batch-shaped; Hooks::before/side_effects/after are the extension points for steps 5, 6, 10 (NoHooks today); renders and validates again after hooks. API: POST/GET /api/notes?type=, GET/PUT/DELETE /api/notes/{id}, /history, /history/{version}, POST /restore, GET /api/trash, POST /api/trash/{id}/restore. JSON body {type?, base_version?, note|wcl}; WCL body uses If-Match; Accept text/plain returns raw WCL with ETag. Missing base version = 428. Auth stub api::Caller always Session("owner"). Assumptions: server runs Vault::init and a full index rebuild on start; notes on disk with no version in state.db are adopted on first access as Session("vault"); JSON field order inside blocks is alphabetical (serde_json without preserve_order); schema is loaded at start (Writer::reload_schema exists for the types ticket); added ulid = 1 dependency.

### 2026-10-12T13:05:45Z — wil

Merged to main as a fast-forward: fc672e3..5619b15.
