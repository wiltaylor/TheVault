//! The integration tests of `state`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p state --test main <module>::`.

mod activity;
mod changes;
mod commands;
mod devices;
mod sessions;
mod shipped;
mod state;
