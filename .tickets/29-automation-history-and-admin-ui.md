---
id: '29'
title: Automation, history and admin UI
plan: thevault-prd
blocked_by:
- '19'
- '15'
- '22'
status: done
verified: verified
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

### 2026-10-12T14:16:02Z — /work

Claimed.

### 2026-10-12T14:34:14Z — wil

Verified set to verified.

### 2026-10-12T14:34:14Z — wil

Status set to done.

### 2026-10-12T14:34:14Z — wil

Done on ticket/thevault-prd-29 (ee27cc0). All four S5 criteria pass in web/e2e/admin.spec.ts (5 tests); full e2e suite 41/41 and ./dev just check green. Not built here: the tile URL setting for maps. Ticket 26 (in progress on its own branch) builds a Settings page with the tile URL, and its acceptance criterion covers it; building a second one here would collide. Script validation: PUT/GET /api/scripts/{name} JSON now carries problems [{role, message}] from Host::check via a new writer Hooks::check_script; scripts are still stored as sent. The sidebar is now a div holding two navs, Types and Vault, so the e2e locators scoped to the Types nav do not also match the Vault 'Tasks' link. e2e global-setup now writes a 32-byte secret key file for the server.

### 2026-10-12T15:12:21Z — wil

Merged into main as b594ad7 (fast-forward). After merging main in, ./dev just check passed, and the full e2e suite passed 57/57 serially and in parallel once load was low. Earlier parallel runs at load average 16-18 timed out broadly (logins and saves), including one 30s hang saving a script; it did not reproduce in 25 S1 loops or at normal load. Worth watching if script saves are ever slow.
