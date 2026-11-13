//! The integration tests of `state`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p state --test main <module>::`.

/// The schema version the current build migrates every older `state.db` to.
const EXPECTED_SCHEMA_VERSION: i64 = 33;
mod automation;
mod run_provenance;

mod activity;
mod ai_usage;
mod approval_quota;
mod changes;
mod classifiers;
mod commands;
mod compressed_history;
mod devices;
mod diagnostics;
mod gitsync;
mod login_origins;
mod sessions;
mod shipped;
mod state;
mod thread_history;
