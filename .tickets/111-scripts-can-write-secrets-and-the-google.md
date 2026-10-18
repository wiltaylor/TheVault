---
id: '111'
title: Scripts can write secrets, and the Google sync keeps its settings and state
  in secrets instead of a google_sync note type
blocked_by: []
status: done
verified: verified
---

# Scripts can write secrets, and the Google sync keeps its settings and state in secrets instead of a google_sync note type

The user decided (2026-10-18, superseding google-sync D6 and the part of conversation Q5 that kept the `secrets` module read-only): scripts may write secrets, and the Google sync keeps its settings and sync state in secrets instead of a `google_sync` settings note. Goals: no note type for sync plumbing, no note history entry per run.

Spec context: /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (read from that absolute path) and MAP.md beside it (D21 records this decision). Shipped code to change: crates/writer/defaults/google_sync.wcl (type to remove), crates/writer/defaults/google/*.ws (google_gmail, google_calendar, google_event scripts and the two task files), crates/api integrations routes (ticket 96), web/src/pages/Integrations*.tsx, crates/script (secrets module, crates/script/src/lib.rs ~120-137), crates/script-core (Vault trait `secret`), crates/state/src/secrets.rs, docs/scripts.md, docs/vault-format.md, docs/google.md.

Build:
1. Runtime: `secrets::set(name: string, value: string) -> Result[(), string]` in the `secrets` module, available where `secrets` is (scheduled tasks and `after_*` handlers), refused for any name not in the run's `secrets` list, stored encrypted like PUT /api/secrets. Value size limit: 64 KiB. Document in docs/scripts.md.
2. Secrets replacing the settings note (each a JSON object string):
   - `google_settings`: { gmail_mode, gmail_labels, gmail_window_days, gmail_backfill_to, calendars, default_calendar, endpoints } with the defaults the google_sync type had. `endpoints` remains the test seam (absent means Google's real hosts).
   - `google_gmail_state`: { history_id, backfill_page, last_run, last_error, notified_invalid_grant }.
   - `google_calendar_state`: { sync_tokens, window_end, last_run, last_error, notified_invalid_grant }.
   - `google_account`: the connected address (plain string).
   Each task writes only its own state secret, so the two tasks never overwrite each other. The event write-back handler reads `google_settings` (default calendar, endpoints) and writes nothing but notes.
   The task files' `secrets` lists name exactly what each script reads and writes.
3. Scripts: read settings and state from the secrets (a missing google_settings means defaults; no refresh token means the run ends successfully with a log line, as a missing settings note did); write state with secrets::set instead of vault::patch on a settings note. Update SHIPPED_FILES hashes.
4. Integrations: the server routes read and write these secrets. GET /api/integrations/google returns connected, account, the settings, and each side's last run, last error and paused. A new PUT /api/integrations/google/settings saves the settings (validated: mode labels|all, window 1-3650 days, date format, non-empty calendars). Connect finish writes google_account and google_settings (keeping endpoints the start used). Disconnect deletes the three credential secrets and google_account, keeps google_settings, clears both state secrets' cursors. The web Google card gains a settings form: mode, labels (chips), window days, back-fill date, calendars (chips), default calendar, Save.
5. Remove the built-in `google_sync` type from the shipped defaults, and add upgrade step `0004_remove_google_sync` in crates/writer/src/shipped.rs UPGRADE_STEPS: when the vault has a google_sync type, move the first google_sync note's settings, cursors and account into the four secrets (only keys not already set), delete every google_sync note permanently (not trash), and delete the type. The step needs the secrets key; on a device (no secrets) it does nothing (steps run on the home server only).
6. Docs: docs/google.md (settings are on the Integrations page), docs/vault-format.md (remove google_sync, describe step 0004), docs/scripts.md (secrets::set, the Google secrets).

Acceptance criteria:
- Script runtime test: secrets::set stores a value a later run reads; a name not in the run's list is refused; over 64 KiB is refused; a before_* handler cannot use it.
- Existing Gmail, Calendar, write-back and connect test suites pass against the secrets layout (their setup now writes google_settings with endpoints instead of a note).
- A test that a Gmail run and a Calendar run each change only their own state secret, and that no note history entry is written for cursor moves.
- Settings form: web component test saving settings; server test of PUT validation.
- Upgrade test: a 0.1.7 vault with a google_sync type and a settings note holding labels and a history id ends with the four secrets holding those values, no google_sync note or type, and a second start does nothing; a 0.1.7 vault with the type and no note just loses the type.
- ./dev just check and ./dev just android-test pass.

## Worklog

### 2026-10-18T02:44:38Z — /work

Claimed.

### 2026-10-18T03:10:51Z — wil

Verified set to verified.

### 2026-10-18T03:10:51Z — wil

Status set to done.

### 2026-10-18T03:10:51Z — wil

Branch ticket/111 commit 9e1483f. Design notes: step 0004 defers via writer::Error::StepDeferred (logged, not recorded, later steps still run) when a google_sync note exists and no secrets key is loaded; with no note it just deletes the type (no key needed). It also adds google_settings to the event type's google_event handler secrets, since the write-back now reads it. Notes are deleted with Writer::delete then State::purge_from_trash. Handlers' before hook now no-ops before Handlers::start (as after already did), so start-up upgrade steps can delete notes. sync_tokens are keyed by calendar id; the step decodes 0.1.7 c_ keys. The write-back handler on invalid_grant only logs (no notify, no write); the calendar task notifies once via notified_invalid_grant. GET status hides settings.endpoints. TestServer::secret/secret_json read secrets from state.db for tests. e2e google-connect.spec.ts updated and passed locally.
