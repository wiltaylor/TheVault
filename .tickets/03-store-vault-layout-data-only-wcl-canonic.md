---
id: '03'
title: 'store: vault layout, data-only WCL, canonical writes, atomic commits'
plan: thevault-prd
blocked_by:
- '01'
- '02'
status: done
verified: verified
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

### 2026-10-12T12:25:46Z — /work

Claimed.

### 2026-10-12T12:36:33Z — wil

Verified set to verified.

### 2026-10-12T12:36:33Z — wil

Status set to done.

### 2026-10-12T12:36:33Z — wil

Branch ticket/thevault-prd-03, commit e45e8ee. Verify: ./dev cargo test -p store (24 tests across tests/{data_only,sandbox,canonical,commit,layout}.rs); ./dev just check exit 0. Decisions taken on the branch: (1) canonical form is printed by store itself, not wcl_lang::format::to_source, because to_source neither aligns '=' nor indents heredocs like docs/vault-format/ does. Rules: fields before blocks; '=' aligned per body; blank line before each block except the first item, between two body-less blocks, and inside a template 'tab' body (the examples write tabs compactly); multi-line strings as <<'END' heredocs at the field's indent; floats always with '.' and never an exponent (WCL lexes 1e21 as a unit literal). Every example under docs/vault-format/vault round-trips byte-for-byte. (2) Commit = redo journal in .vault/txn: staged files + manifest.json rename as commit point; recovery on open and before every commit. Fault injection is the doc(hidden) Vault::commit_with_hook; tests stop at every step in-process and also abort a child process. (3) Commit paths must sit under a layout folder; notes/types/taxonomies/tasks need .wcl, scripts .ws; .vault/ is never writable or loadable. (4) Field order within a note (type order, unknown fields last) needs the type and is left to writer/schema. Mutation-checked: disabling recovery fails the commit tests; opening the loader fails the sandbox tests.

### 2026-10-12T12:37:06Z — wil

Merged into main by fast-forward to e45e8ee (main had not moved since the branch point; just check had passed on that commit).
