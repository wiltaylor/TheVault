---
id: '34'
title: writer does not round ink stroke coordinates and pressure as docs/vault-format.md
  says
blocked_by: []
status: done
verified: verified
---

# writer does not round ink stroke coordinates and pressure as docs/vault-format.md says

## Worklog

### 2026-10-12T14:40:20Z — wil

Triage (orchestrator): make the writer match docs/vault-format.md. Acceptance: a test writes unrounded ink coordinates and pressure and the stored file holds the rounded values the format document specifies.

### 2026-10-12T14:40:20Z — wil

Status set to todo.

### 2026-10-12T15:12:49Z — /work

Claimed.

### 2026-10-12T15:16:08Z — wil

Verified set to verified.

### 2026-10-12T15:16:08Z — wil

Status set to done.

### 2026-10-12T15:16:09Z — wil

Fixed in note::print (crates/writer/src/note.rs): the points list of every stroke block is rounded, x/y to 2 places and pressure to 3, so ink fields, ink content blocks and canvas ink nodes all round. Only Float items are rounded; size is left alone because the format names only coordinates and pressure. Test: crates/writer/tests/ink.rs creates from JSON with unrounded points in all three places and updates from WCL, then reads the stored file. Note 20.005 is 20.00499... in f64 and rounds to 20.0, so the test uses 20.006. ./dev just check passes.
