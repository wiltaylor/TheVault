//! The integration tests of `sync`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p sync --test main <module>::`.

mod asset_transfers;
mod progress;
mod two_node;
