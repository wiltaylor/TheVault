//! The integration tests of `schema`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p schema --test main <module>::`.

mod agents;
mod columns;
mod controls;
mod list_page;
mod period_calendar;
mod skills;
mod tables;
mod time;
mod toggles;
mod validation;
mod when;
