---
id: '03'
title: 'store: vault layout, data-only WCL, canonical writes, atomic commits'
plan: thevault-prd
blocked_by:
- '01'
- '02'
status: todo
verified: unknown
---

# store: vault layout, data-only WCL, canonical writes, atomic commits

Implement the `store` crate.

Scope:
- Open a vault folder with the D22 layout. Create the layout on init.
- Load WCL through a loader that cannot read outside the vault folder.
- Reject any file containing an expression or an import, with a diagnostic naming the item (D20).
- Write every file in canonical format; comments are dropped (D21).
- Atomic multi-file commits: a set of file writes and deletes lands entirely or not at all, including across a crash; recovery runs on open.

Acceptance criteria:
- Loading or submitting WCL with an expression or an import fails with a diagnostic naming it.
- An import or path that points outside the vault cannot read the outside file.
- Writing a non-canonical but valid file produces the canonical text, and writing it twice is byte-identical.
- A test that kills the commit part way (fault injection between file writes) and reopens the vault finds either all or none of the files changed.
- Every example in `docs/vault-format/` round-trips through load and canonical write.

Seam: S3 (files on disk), plus unit tests in the crate. Decisions D2, D20, D21, D22, D23.

## Worklog
