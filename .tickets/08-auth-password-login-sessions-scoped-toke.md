---
id: 08
title: 'auth: password login, sessions, scoped tokens'
plan: thevault-prd
blocked_by:
- '07'
status: done
verified: verified
---

# auth: password login, sessions, scoped tokens

Implement authentication and authorisation in `api`.

Scope:
- Password login creating a session cookie; logout; Argon2id check against `state` (D11, CLAUDE.md OQ-S3).
- Bearer API tokens with scopes `read`, `write`, `admin`, checked per request (D11). `admin` alone may change types, templates, taxonomies, scripts, tasks, secrets and tokens.
- Token management endpoints (admin): create (secret shown once), list, revoke.
- Every endpoint is behind auth except health and login.

Acceptance criteria (S1):
- Without credentials every non-public endpoint returns 401.
- A `read` token can read a note and gets 403 on write.
- A `write` token can write a note and gets 403 on changing a type.
- An `admin` token can change a type.
- A revoked token gets 401.
- Login with the wrong password returns 401; the right one sets a cookie that authorises reads.

Seam: S1. Decisions D11.

## Worklog

### 2026-10-12T13:05:19Z — /work

Claimed.

### 2026-10-12T13:11:56Z — wil

Verified set to verified.

### 2026-10-12T13:11:56Z — wil

Status set to done.

### 2026-10-12T13:11:56Z — wil

Branch ticket/thevault-prd-08 is based on ticket/thevault-prd-07 (5619b15), which is done but not merged into main yet; 07 is itself based on 06. Merge 06 and 07 first. Criteria are proven by crates/server/tests/auth.rs (7 S1 tests). Mutation check: disabling the scope comparison in api::auth::authenticate fails the read-403 and write-403 tests. Design: the auth::authenticate middleware sits on the whole router. auth::required_scope is the policy table: health and POST /api/login are public; /api/tokens and /api/secrets need admin; GET and HEAD need read; non-GET under /api/notes, /api/trash and /api/assets needs write; any other change needs admin. Later tickets that add write-scoped routes (batch, upsert, assets) must add them to WRITE_PATHS. Read-only POSTs such as a query endpoint need a policy entry too, or they will default to admin. Sessions are in memory (30 days, lost on restart), act as Author::Session("owner") with admin scope, and use the cookie tv_session (HttpOnly, SameSite=Strict, no Secure flag because the server is LAN-first over http). Tokens author as Author::Token(name). testkit now seeds testkit::PASSWORD and an admin token (TestServer::admin_token) into .vault/state.db before start. tests/notes.rs sends that token, so its history author is now token/admin. Assumption: the criterion 'a write token gets 403 on changing a type' needed a type endpoint, so PUT /api/types/{name} (admin) was added through Writer::put_type. It creates a type, or replaces one that has no notes, and answers 409 type_in_use otherwise. Ticket 10 owns the full plan/apply flow and should replace or extend it. argon2 and blake2 build at opt-level 3 in the dev profile because debug Argon2 is slow. Left alone: no login rate limiting, and no static web client paths yet. Every non-public path requires auth, so the web-serving ticket must add its public paths to required_scope.

### 2026-10-12T13:12:44Z — wil

Merged into main at 541b5ba (fast-forward) after merging main (07, 11) into the branch; ./dev just check passed on the merged tree.
