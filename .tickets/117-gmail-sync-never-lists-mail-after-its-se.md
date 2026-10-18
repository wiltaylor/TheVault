---
id: '117'
title: 'Gmail sync never lists mail after its settings change: a first run with no
  labels saves a history cursor, and later runs only read history, so switching to
  all mail or adding labels imports nothing'
blocked_by: []
status: done
verified: verified
---

# Gmail sync never lists mail after its settings change: a first run with no labels saves a history cursor, and later runs only read history, so switching to all mail or adding labels imports nothing

Found on the live server (0.1.8), 2026-10-18. Run 1 ran in labels mode with no labels, listed nothing, and saved history_id from the profile. The user then set gmail_mode = all (back-fill 2026-08-01). Runs 4 and 7 logged 'Gmail: 0 new, 0 relabelled, 0 archived, 0 not saved.' with no error: sync() in crates/writer/defaults/google/google_gmail.ws only does full_list when history_id is empty, and incremental history only covers changes after the cursor. Worked around on the live server by PUT /api/secrets/google_gmail_state with '{}'.

Fix:
1. The Gmail state records what the window listing covered: listed = { mode, labels (sorted ids), window_days }. A run whose current settings differ lists again (the window for the new mode and labels; already-imported messages skip by gmail_ref), keeps history for incremental changes, and stores the new listed value. Adding a label lists only that label; removing one lists nothing.
2. A run with nothing to list (labels mode, no labels) saves no history_id, so the first real listing happens once settings exist.
3. A back-fill date inside the window is reported once in the log ('back-fill date is inside the window; nothing older to fetch') rather than silently done.
4. The Integrations settings form notes under the labels that changing labels or mode lists the window again on the next run.

Acceptance criteria (server tests through TestServer and FakeGoogle): first run with no labels then labels added imports those labels' window; switching labels to all imports the window; adding a label imports only its messages; nothing duplicates; ./dev just check passes.

## Worklog

### 2026-10-18T04:13:57Z — /work

Claimed.

### 2026-10-18T04:22:57Z — wil

Verified set to verified.

### 2026-10-18T04:22:57Z — wil

Status set to done.

### 2026-10-18T04:22:57Z — wil

Done on ticket/117. State gains listed {mode, labels (sorted ids), window_days}; relist_of() picks all lists for a new mode/window or a missing record (legacy states with a history_id and no listed relist once), else only added labels. Back-fill cursor resets whenever listed differs, so adding a label re-pages the back-fill for every label (skipped by gmail_ref, costs list requests only). Disconnect (crates/api/src/google.rs) now nulls listed too. New hash a4f14497... appended to SHIPPED_FILES. Tests: 5 new in crates/server/tests/google_gmail.rs, one assertion in web/src/pages/Integrations.test.tsx.
