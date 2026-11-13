//! The integration tests of `gitsync`, linked as one test binary. Run one module with
//! `cargo test -p gitsync --test main <module>::`.

mod context;
mod crypto;
mod freshness;
mod git_env;
mod git_file;
mod git_http;
mod git_ssh;
mod instance;
mod layout;
mod manifest;
mod pack;
mod remote_https;
mod remote_ssh;
mod repo_layout;
mod rounds;
mod scheduled;
mod sshkey;
mod three;
