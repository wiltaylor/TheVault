---
id: '52'
title: 'script-core: split the wscript engine out of crates/script'
plan: thevault-clients
blocked_by: []
status: todo
verified: unknown
---

# script-core: split the wscript engine out of crates/script

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement the SPEC section "Script engine on the device" groundwork (D6): move Kind, Job, Limits, Host and the `Vault` trait into a new crate `crates/script-core` with no dependency on writer, state or rusqlite. `crates/script` keeps server implementations (vault modules backed by Writer, http, fs, secrets, notify) and re-exports the engine. Add a `Kind::Device` with its module set placeholder (the `device` module itself is a later ticket).

Acceptance criteria (S1 and existing tests):
- `cargo tree -p script-core` shows no writer, state, rusqlite, reqwest or rustix.
- Every existing Rust test passes unchanged; `./dev just check` passes.
- A unit test in script-core runs a script against an in-memory test `Vault` implementation.
Decisions: D6.

## Worklog
