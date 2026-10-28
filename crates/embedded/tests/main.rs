//! The integration tests of `embedded`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p embedded --test main <module>::`.

mod device;
mod handle;
mod home;
mod import;
mod subagents;
