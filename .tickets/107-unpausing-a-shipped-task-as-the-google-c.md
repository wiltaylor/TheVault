---
id: '107'
title: Unpausing a shipped task (as the Google connect route will for google_gmail
  and google_calendar) rewrites tasks/<name>.wcl, so its hash no longer matches a
  shipped version and every later shipped version of the task is kept with a 'newer
  version' notice instead of upgrading; pause state may need to live outside the shipped
  file
blocked_by: []
status: done
verified: verified
---

# Unpausing a shipped task (as the Google connect route will for google_gmail and google_calendar) rewrites tasks/<name>.wcl, so its hash no longer matches a shipped version and every later shipped version of the task is kept with a 'newer version' notice instead of upgrading; pause state may need to live outside the shipped file

## Worklog

### 2026-10-18T00:23:19Z — /work

Claimed.

### 2026-10-18T00:32:27Z — wil

Verified set to verified.

### 2026-10-18T00:32:27Z — wil

Status set to done.

### 2026-10-18T00:32:27Z — wil

Fixed on ticket/107: shipped_hash (crates/writer/src/shipped.rs) hashes task files without top-level paused via the WCL parser; upgrade carries the vault's paused via with_paused_of; the two task hashes in SHIPPED_FILES were replaced by their paused-less form (the old full hashes could never match a normalised disk hash). A state.db record holding the old plain hash of the current text is treated as current for the notify check. Tests in crates/writer/tests/shipped_defaults.rs use Writer::seed_shipped_defaults_with with a newer gmail task (timeout PT10M). Ticket 108 is the duplicate.
