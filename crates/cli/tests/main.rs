//! The integration tests of `cli`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p cli --test main <module>::`.

mod agent_install;
mod cli;
