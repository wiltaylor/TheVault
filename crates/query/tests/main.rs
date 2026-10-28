//! The integration tests of `query`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p query --test main <module>::`.

mod query;
mod when;
