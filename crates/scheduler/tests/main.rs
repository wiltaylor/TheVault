//! The integration tests of `scheduler`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p scheduler --test main <module>::`.

mod agents;
mod ai;
