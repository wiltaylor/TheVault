---
id: '59'
title: 'crates/embedded: the in-process server for the apps'
plan: thevault-clients
blocked_by:
- '52'
status: todo
verified: unknown
---

# crates/embedded: the in-process server for the apps

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC "Repository layout" crates/embedded and the local-server rules (D17, D18, D26): a library that opens a local vault directory, builds the existing api router (same handlers as crates/server) with the scheduler and the after_* handler queue disabled, and exposes `handle(request) -> response` for an in-process caller (a Tauri custom scheme handler), treating every request as the device owner (no session needed). It serves web/dist for non-/api paths like crates/server's web fallback. Must support Range requests and streaming uploads through `handle` (pdf.js) - test it. No Tauri dependency in this crate.

Acceptance criteria:
- Rust tests drive `handle` directly: create, read, update notes; search; asset upload and a ranged read returning 206; static file serving with SPA fallback; no scheduler thread or handler queue runs (an after_* handler declared in the test vault does not fire).
- `./dev just check` passes.
Decisions: D17, D18, D26.

## Worklog
