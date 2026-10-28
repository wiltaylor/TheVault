//! The integration tests of `bench`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p bench --test main <module>::`.

mod generate;
