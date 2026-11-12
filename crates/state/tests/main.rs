//! The integration tests of `state`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p state --test main <module>::`.

mod activity;
mod ai_usage;
mod approval_quota;
mod changes;
mod classifiers;
mod commands;
mod devices;
mod gitsync;
mod login_origins;
mod sessions;
mod shipped;
mod state;
mod thread_history;
