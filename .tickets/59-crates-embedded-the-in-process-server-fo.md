---
id: '59'
title: 'crates/embedded: the in-process server for the apps'
plan: thevault-clients
blocked_by:
- '52'
status: done
verified: verified
---

# crates/embedded: the in-process server for the apps

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Repository layout" crates/embedded and the local-server rules (D17, D18, D26): a library that opens a local vault directory, builds the existing api router (same handlers as crates/server) with the scheduler and the after_* handler queue disabled, and exposes `handle(request) -> response` for an in-process caller (a Tauri custom scheme handler), treating every request as the device owner (no session needed). It serves web/dist for non-/api paths like crates/server's web fallback. Must support Range requests and streaming uploads through `handle` (pdf.js) - test it. No Tauri dependency in this crate.

Acceptance criteria:
- Rust tests drive `handle` directly: create, read, update notes; search; asset upload and a ranged read returning 206; static file serving with SPA fallback; no scheduler thread or handler queue runs (an after_* handler declared in the test vault does not fire).
- `./dev just check` passes.
Decisions: D17, D18, D26.

## Worklog

### 2026-10-15T06:26:32Z — /work

Claimed.

### 2026-10-15T06:39:22Z — wil

Verified set to verified.

### 2026-10-15T06:39:22Z — wil

Status set to done.

### 2026-10-15T06:39:22Z — wil

Branch ticket/thevault-clients-59, commit 78f955d. api gains owner_router (every request is the owner session, middleware auth::as_owner) and pub mod web (moved from crates/server). Embedded::open runs on its own thread and Drop drops parts on a thread: script hosts build reqwest::blocking clients, which panic when created or dropped inside a Tokio runtime. after_* is dropped by a LocalHooks wrapper and Handlers::bind (not start) is used, so nothing is queued in state.db. POST /api/tasks/{name}/run answers 409 home_server_only (assumption: a manual task run counts as a task on the device, D17). Web serving is path-based (Config::web: PathBuf); Android's bundled assets may need a different source later. Tests: crates/embedded/tests/handle.rs, 5 tests; the after-handler test was mutation-checked (re-enabling the hook and queue makes it fail).
