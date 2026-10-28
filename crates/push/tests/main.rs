//! The integration tests of `push`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p push --test main <module>::`.

mod dispatch;
