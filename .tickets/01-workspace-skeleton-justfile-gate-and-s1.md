---
id: '01'
title: Workspace skeleton, justfile gate and S1 testkit
plan: thevault-prd
blocked_by: []
status: todo
verified: unknown
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
