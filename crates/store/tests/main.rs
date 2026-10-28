//! The integration tests of `store`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p store --test main <module>::`.

mod canonical;
mod commit;
mod data_only;
mod layout;
mod sandbox;
mod skills;
