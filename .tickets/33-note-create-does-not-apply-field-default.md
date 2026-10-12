---
id: '33'
title: Note create does not apply field defaults
blocked_by: []
status: done
verified: verified
---

# Note create does not apply field defaults

docs/vault-format.md says a field's `default` is 'the value a new note gets'. POST /api/notes does not apply it: creating a note of a type with a required field that has a default, without sending the field, fails validation with '<field>: is required'. Found while working ticket 10 (schema changes), which applies defaults to existing notes only. Either the writer should fill defaults on create, or the doc means client-side prefill; needs a decision.

## Worklog

### 2026-10-12T14:40:20Z — wil

Triage (orchestrator): the server fills defaults on create. Writer applies each field's default for fields the create request omits, before validation. Acceptance (S1): creating a note without a defaulted required field succeeds and the file holds the default; an explicit value wins over the default; update requests do not re-apply defaults.

### 2026-10-12T14:40:20Z — wil

Status set to todo.

### 2026-10-12T14:41:01Z — /work

Claimed.

### 2026-10-12T15:09:55Z — wil

Verified set to verified.

### 2026-10-12T15:09:55Z — wil

Status set to done.

### 2026-10-12T15:09:55Z — wil

Fix: Writer::prepare_put fills ty.fields defaults when old is None, after the computed-write check and before arrange. Covers create, stage_create (handlers), batch create and upsert-create. It also covers restore_from_trash (old=None): a restored note missing a defaulted field gets the default — judged harmless, left as is. Test: crates/server/tests/notes.rs create_fills_field_defaults_and_update_does_not. Seen once under load: script/tests/handlers.rs after_jobs_queued_when_the_server_stops_run_when_it_starts_again timed out (20s deadline); passed alone and on the gate rerun.

### 2026-10-12T15:15:29Z — wil

Merged into main at 1e77ed0 (fast-forward, after merging main into ticket/33; gate green on the merged tree).
