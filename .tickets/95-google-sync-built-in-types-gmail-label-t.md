---
id: '95'
title: Google sync built-in types, gmail_label taxonomy, and the fake Google server
  in testkit
blocked_by:
- '94'
status: claimed
verified: unknown
---

# Google sync built-in types, gmail_label taxonomy, and the fake Google server in testkit

The spec is /home/wil/dev/TheVault/.scratch/plans/google-sync/SPEC.md (gitignored, in the main checkout; read it from that absolute path, since worktrees do not have .scratch). Decision ids D1-D20 there are the google-sync map's, at /home/wil/dev/TheVault/.scratch/plans/google-sync/MAP.md, with reasoning in its notes/; cite them in commits as 'google-sync Dn'. Research on Google's OAuth and API behaviour: /home/wil/dev/TheVault/.scratch/plans/google-sync/notes/01-google-oauth-constraints.md.

Scope: the 'Built-in types and taxonomy' section of the spec, shipped through crates/writer/src/shipped.rs (ticket 86's mechanism), plus the fake Google server from 'Testing decisions' with its base (port 0, token endpoint with refresh and code exchange and an invalid_grant switch, userinfo) laid out as separate modules per API (for example crates/testkit/src/fake_google/{mod,oauth,gmail,calendar}.rs) so tickets for Gmail, Calendar and the connect route each extend their own module without conflicting. Gmail and Calendar endpoints are added by their own tickets; leave gmail.rs and calendar.rs as empty modules with a doc comment. Do not ship tasks or scripts here (each sync ticket ships its own). The event type's handler declaration is added by the write-back ticket, not here. The event type needs date_range of = "date_or_datetime" from the schema ticket, which this ticket is blocked by.

Acceptance criteria:
- Upgrade test: a 0.1.6 vault gains email, email_thread, event, google_sync and gmail_label on start, a second start changes nothing.
- Type tests: each type's fields match the spec, the templates render (an existing template test seam), and google_sync's endpoints field is hidden.
- A testkit test exercises the fake's token refresh, code exchange, invalid_grant switch and userinfo over HTTP.
- ./dev just check passes.

## Worklog

### 2026-10-17T23:10:48Z — /work

Claimed.
