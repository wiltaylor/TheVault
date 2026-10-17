---
id: '96'
title: 'Settings > Integrations > Google: connect with the user''s OAuth client (loopback
  paste or https direct), status, Disconnect, and docs/google.md'
blocked_by:
- '95'
status: done
verified: verified
---

# Settings > Integrations > Google: connect with the user's OAuth client (loopback paste or https direct), status, Disconnect, and docs/google.md

The spec is /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (gitignored, in the main checkout; read it from that absolute path, since worktrees do not have .scratch). Decision ids D1-D20 there are the google-sync map's, at /home/wil/dev/TheVault/.scratch/plans/google-sync/MAP.md, with reasoning in its notes/; cite them in commits as 'google-sync Dn'. Research on Google's OAuth and API behaviour: /home/wil/dev/TheVault/.scratch/plans/google-sync/notes/01-google-oauth-constraints.md.

Scope: the 'Connect route and Integrations page' section of the spec, plus docs/google.md. Extend the fake Google server's oauth module as needed.

Acceptance criteria:
- Server tests against the fake: start returns an auth_url with the scopes, access_type=offline, prompt=consent, PKCE and state; finish with a pasted loopback URL stores google_refresh_token, writes account, creates the settings note and unpauses both tasks (tasks may not exist yet when this ticket runs alone; unpause whichever exist and test it with a task file the test writes); a wrong state is refused; disconnect deletes the secrets and pauses the tasks; GET reports the state.
- Web component tests for the loopback paste flow, the https direct flow (SPA callback route), the connected state and Disconnect.
- A Playwright check of the card against a server whose settings note endpoints point at the fake; save a screenshot to /home/wil/dev/TheVault/.scratch/shots/ticket-google-connect.png.
- docs/google.md walks through the Google Cloud setup as the spec lists.
- ./dev just check passes.

## Worklog

### 2026-10-17T23:29:37Z — /work

Claimed.

### 2026-10-17T23:47:33Z — wil

Verified set to verified.

### 2026-10-17T23:47:33Z — wil

Status set to done.

### 2026-10-17T23:47:33Z — wil

Committed 9f499f7 on ticket/96. Server: crates/api/src/google.rs (routes on their own Routes group, /api/integrations admin-only via ADMIN_PATHS), Scheduler::set_paused (scheduler::with_paused text edit). Tests: crates/server/tests/google_connect.rs (8, against FakeGoogle), web/src/pages/Integrations.test.tsx (9), web/e2e/google-connect.spec.ts (spawns new testkit-fake-google bin; fake gained an HTTP consent page GET /o/oauth2/v2/auth -> 302, re-exported AUTH_PATH in fake_google/mod.rs). Choices: the server writes account as Author::Task(SERVER_AUTHOR) because it is computed; a flow keeps the endpoints it started with and a google_sync note made by finish carries that endpoints dict; pending sign-ins are in memory (a restart loses them); the auth URL is always accounts.google.com (endpoints has no auth key). Tasks google_gmail/google_calendar: tests write their own with a noop script; on merge with 97/98 nothing should conflict except that the e2e spec overwrites the shipped task files on its shared server with noop ones. theme-settings.spec.ts updated for the new Integrations tab. ./dev just check exit 0; ./dev just web-e2e-test 307 passed + the one fixed spec re-run green.
