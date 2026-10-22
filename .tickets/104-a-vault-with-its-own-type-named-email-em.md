---
id: '104'
title: 'A vault with its own type named email, email_thread, event or google_sync
  has it treated as the built-in on start: seeding adds every shipped field it lacks
  (google_ref, snapshot, gmail_ref, endpoints...) and it can no longer be deleted
  (ticket 95 made these names built-in)'
blocked_by: []
status: done
verified: verified
---

# A vault with its own type named email, email_thread, event or google_sync has it treated as the built-in on start: seeding adds every shipped field it lacks (google_ref, snapshot, gmail_ref, endpoints...) and it can no longer be deleted (ticket 95 made these names built-in)

## Worklog

### 2026-10-22T02:09:27Z — wil

Status set to todo.

### 2026-10-22T02:10:06Z — wil

Claimed.

### 2026-10-22T02:21:03Z — wil

Done on ticket/148 (3a986d0), with 148. Rule: a vault type holding a built-in's name is the built-in when state.db records the server writing it (shipped_files row under types/<name>.wcl, written when seeding or ensure_built_in_types adds it) or when it declares the built-in's key field as a single string field (message/call device_ref, conversation/email_thread thread_ref, email gmail_ref, event google_ref; every shipped version has it); a type found by identity is recorded on that start. dailynote and fleeting are always built-in. Writer::is_built_in drives delete_type and the API builtin flag (api types.rs/inspect.rs); seeding skips a user type with one eprintln per start and Upgraded.types_skipped; step 0003 skips a user event; step 0004 keeps a google_sync type without gmail_history_id. Not covered: the Google and phone syncs still write into whatever type holds the name. Tests in crates/writer/tests/shipped_defaults.rs and crates/server/tests/shipped_defaults.rs; docs/vault-format.md step 2. ./dev just check passes.

### 2026-10-22T02:21:04Z — wil

Verified set to verified.

### 2026-10-22T02:21:04Z — wil

Status set to done.
