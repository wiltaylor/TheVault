//! The integration tests of `script`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p script --test main <module>::`.

mod ai;
mod handlers;
mod host;
mod task_api;
mod wscript_skill;
