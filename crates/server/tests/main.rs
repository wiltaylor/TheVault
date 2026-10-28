//! The integration tests of `server`, linked as one test binary: each module was its own
//! binary, and linking them all saturated the disk. Run one module with
//! `cargo test -p server --test main <module>::`.

/// Office documents built in code, shared with the index and writer tests.
#[path = "../../index/tests/support/office.rs"]
mod office;

mod activity;
mod agent_files;
mod agent_memory;
mod agent_mentions;
mod agent_threads;
mod agent_triggers;
mod agents;
mod ai_agents;
mod ai_browser;
mod ai_chat;
mod ai_outbox;
mod ai_settings;
mod ai_skill_changes;
mod ai_skills;
mod ai_taxonomies;
mod ai_web;
mod archive;
mod asset_memory;
mod asset_usage;
mod auth;
mod background_turns;
mod brave;
mod browser_desktop;
mod canvas;
mod client_ids;
mod clients;
mod completeness;
mod daily;
mod device_commands;
mod device_runtime;
mod devices;
mod event_series;
mod events;
mod fleeting;
mod google_calendar;
mod google_connect;
mod google_event;
mod google_gmail;
mod google_types;
mod graph;
mod handlers;
mod health;
mod notes;
mod notifications;
mod office_assets;
mod outbox_email;
mod outbox_event;
mod outbox_notifications;
mod outbox_sms;
mod pdf_assets;
mod phone_types;
mod pins;
mod planner;
mod push;
mod query;
mod refs;
mod saved_scripts;
mod scheduler;
mod schema_changes;
mod script_tools;
mod shipped_defaults;
mod skills;
mod subagents;
mod sync;
mod trash;
mod web;
