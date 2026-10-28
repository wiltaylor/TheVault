//! The integration tests of `testkit`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p testkit --test main <module>::`.

mod fake_ai;
mod fake_calendar;
mod fake_google;
