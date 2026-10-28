//! The integration tests of `browser-agent`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p browser-agent --test main <module>::`.

mod api;
