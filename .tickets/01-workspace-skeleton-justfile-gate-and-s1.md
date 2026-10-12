---
id: '01'
title: Workspace skeleton, justfile gate and S1 testkit
plan: thevault-prd
blocked_by: []
status: done
verified: verified
---

# Workspace skeleton, justfile gate and S1 testkit

Create the Cargo workspace and the web app skeleton every other ticket builds on.

Scope:
- Cargo workspace with crates `store`, `schema`, `writer`, `index`, `state`, `query`, `script`, `scheduler`, `api`, `cli` (binary `vault`), `server` (binary `vault-server`) and `testkit`, as CLAUDE.md lists. Pin `wcl_lang` and `wscript` exactly as CLAUDE.md shows, in `[workspace.dependencies]`.
- `crates/server` serves `GET /api/health` returning `{"status":"ok"}` and takes `--vault <dir>` and `--listen <addr>`, with port 0 allowed. On start it prints one JSON line with the bound address.
- `crates/testkit`: a helper that creates a temporary vault directory, starts `vault-server` on port 0, waits for health, and returns a base URL plus a handle that kills the server on drop. This is seam S1.
- `web/`: SolidJS 1.9, Vite, TypeScript strict, Vitest, and Playwright configured with the chromium in `/opt/playwright`, using pnpm.
- `justfile` with recipes `check` (fmt check, clippy -D warnings, cargo test, web typecheck, web unit tests), `test-e2e` (Playwright), `fmt`, `server`, `web-dev`.

Acceptance criteria:
- `./dev just check` passes on a clean clone.
- A testkit-based integration test starts the server on a random port and gets `{"status":"ok"}` from `/api/health`.
- Two copies of that test run in parallel without a port clash.
- `./dev pnpm -C web build` produces `web/dist`.
- `./dev just test-e2e` runs one Playwright smoke test that loads the built app.

Spec: SPEC.md Components, Testing decisions S1. Decisions D19.

## Worklog

### 2026-10-12T12:09:43Z — /work

Claimed.

### 2026-10-12T12:14:21Z — wil

Verified set to verified.

### 2026-10-12T12:14:21Z — wil

Status set to done.

### 2026-10-12T12:14:21Z — wil

Branch ticket/thevault-prd-01 @ b03de88. All five criteria shown on a fresh clone: just check exit 0; pnpm build made web/dist; just test-e2e 1 passed; health test binary run twice at once, 4/4 passed each (each copy starts 6 servers on port 0); two Playwright runs at once both passed. Choices other tickets should know: S1 tests live in crates/server/tests, because testkit finds vault-server in the cargo target dir (or VAULT_SERVER_BIN) and cargo only guarantees that binary for the server crate's tests or a workspace run. vault-server refuses a missing --vault dir. The gate parts are in .just/ci (just ci::check); top-level check depends on it. 'server' and 'test-e2e' are aliases for server-run and web-e2e-test. Playwright serves web/dist with vite preview on port 0 from global setup and passes the URL through E2E_BASE_URL. pnpm store-dir=/pnpm-store/store is set in web/.npmrc. store depends on wcl_lang and script on wscript, which proves the pins build. Resolved versions: Vite 8, Vitest 5, TypeScript 7, solid-js 1.9.17, @playwright/test pinned to 1.56.1 to match the browser in the image.

### 2026-10-12T12:15:16Z — wil

Merged into main by fast-forward (07bf833..b03de88).
