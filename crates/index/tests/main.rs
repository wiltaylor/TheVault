//! The integration tests of `index`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p index --test main <module>::`.

/// Office documents and crafted PDFs built in code, and the retry on a busy extractor.
mod support;

mod hostile_pdf;
mod index;
mod office_text;
mod prefix;
mod search;
