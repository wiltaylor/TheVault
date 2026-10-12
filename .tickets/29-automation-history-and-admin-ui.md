---
id: '29'
title: Automation, history and admin UI
plan: thevault-prd
blocked_by:
- '19'
- '15'
- '22'
status: todo
verified: unknown
---

# Automation, history and admin UI

Build the remaining UI so every API action has a UI equivalent.

Scope:
- Script editor for handlers and migrations (CodeMirror) with save and validation errors shown.
- Tasks: list, edit cron and allowlist and secret names, run now, run history with logs, failure indicator.
- Secrets management (write-only values) and token management (create shows secret once, revoke).
- Note history with restore; trash with restore; flagged notes list.
- Tile URL setting for maps.

Acceptance criteria (S5):
- Creating a before_update handler in the UI and saving a note runs it.
- A failing task shows as failed in the UI with its log.
- Restoring a note from history and from trash works from the UI.
- Creating a token shows its secret once; after reload it is not shown.

Seam: S5. Decisions D9, D10, D11, D13.

## Worklog
