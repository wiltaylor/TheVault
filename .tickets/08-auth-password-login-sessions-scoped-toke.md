---
id: 08
title: 'auth: password login, sessions, scoped tokens'
plan: thevault-prd
blocked_by:
- '07'
status: todo
verified: unknown
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
