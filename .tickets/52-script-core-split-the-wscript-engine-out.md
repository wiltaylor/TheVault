---
id: '52'
title: 'script-core: split the wscript engine out of crates/script'
plan: thevault-clients
blocked_by: []
status: done
verified: verified
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

### 2026-10-15T06:06:06Z — /work

Claimed.

### 2026-10-15T06:22:10Z — wil

Verified set to verified.

### 2026-10-15T06:22:10Z — wil

Status set to done.

### 2026-10-15T06:22:10Z — wil

Committed dc0caad on ticket/thevault-clients-52. Two deviations from the ticket's literal shape, forced by the type system: (1) Author and Archived moved to a new dependency-free crate crates/vocab (state and index re-export them) because the Vault trait names both and their old homes pull rusqlite; (2) impl Vault for Writer moved to crates/writer/src/script_vault.rs because the orphan rule forbids it in crates/script once the trait lives in script-core; writer now depends on script-core and query. script::Host is a newtype over script_core::Host (Deref) that registers http/secrets/fs and vault::upload_asset_file via the Extensions trait. Kind::Device has its own registration with no extra modules yet; the device module ticket adds it through Extensions::modules(Kind::Device). First just check run hit a load-induced reqwest timeout in server/tests/completeness.rs; reran clean twice and the full gate passed.
