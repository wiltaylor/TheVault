---
id: '41'
title: 'cli: no command for POST /api/query/text, the query builder''s model-to-text
  call'
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# cli: no command for POST /api/query/text, the query builder's model-to-text call

Found by the ticket 31 parity audit (D40). The visual query builder (web/src/editor/QueryEditor.tsx:70-74) turns its model into text through POST /api/query/text on every edit. vault has query-parse for text-to-model but no command for model-to-text, so the UI action has an API endpoint and no CLI command. A 'vault query-text' taking the model JSON on stdin or -f would close it. See docs/parity.md.

## Worklog

### 2026-10-12T15:56:13Z — wil

Triage (orchestrator): Fix: add vault query to-text and vault query from-text (model JSON <-> text) over the existing endpoints, JSON output. Acceptance (S2): round trip of a builder model through both commands is identical.

### 2026-10-12T15:56:13Z — wil

Status set to todo.

### 2026-10-12T17:21:06Z — /work

Claimed.

### 2026-10-12T18:06:40Z — wil

Verified set to verified.

### 2026-10-12T18:06:43Z — wil

Status set to done.

### 2026-10-12T18:06:46Z — wil

Added vault query-text (crates/cli/src/main.rs) over POST /api/query/text. Input is the model JSON on stdin or -f, or the whole query-parse answer (its model key is taken), so query-parse | query-text pipes. Went with one flat command named query-text next to query-parse rather than the triage note's 'vault query to-text/from-text': vault query <text> is already a positional command, and making it a subcommand group would break it. Test: crates/cli/tests/cli.rs a_builder_model_round_trips_through_query_parse_and_query_text (model -> text -> model identical, file input, non-JSON -> usage error, bad model -> 400); every_command_prints_json covers it too. docs/parity.md rows updated. Gate: fmt, clippy, web typecheck and tests green; every Rust test green except crates/script wscript wall-clock timeouts under load avg ~12, filed as ticket 47 (needs-triage), not touched by this diff.

### 2026-10-12T18:29:27Z — wil

Merged into main at 4c1dada (fast-forward) after merging main 67686b9 into the branch; ./dev just check green on the merged tree, script tests included this time (load avg ~5-8).
