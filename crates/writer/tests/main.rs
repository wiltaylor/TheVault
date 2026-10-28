//! The integration tests of `writer`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p writer --test main <module>::`.

mod activity;
mod agent_memory;
mod agent_threads;
mod agents;
mod ai_accounts;
mod archive;
mod asset_use;
mod atomic;
mod canvas;
mod code_blocks;
mod columns;
mod controls;
mod definition_events;
mod dicts;
mod event_series;
mod ink;
mod office_text;
mod open_taxonomies;
mod outbox;
mod pdf_text;
mod query_views;
mod sessions;
mod shipped_defaults;
mod skills;
mod startup;
mod template_images;
mod toggles;
