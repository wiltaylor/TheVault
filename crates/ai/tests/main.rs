//! The integration tests of `ai`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p ai --test main <module>::`.

/// The shared fixture: a `state.db` with accounts pointed at the testkit fakes.
mod common;

mod anthropic;
mod chat_completions;
mod credentials;
mod custom_url;
mod images;
mod openai;
mod web;
