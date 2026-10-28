//! The integration tests of `script-core`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p script-core --test main <module>::`.

mod memory;
