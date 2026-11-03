//! The integration tests of `gitsync`, linked as one test binary. Run one module with
//! `cargo test -p gitsync --test main <module>::`.

mod crypto;
mod git_env;
mod git_file;
mod git_http;
mod git_ssh;
mod instance;
mod layout;
mod manifest;
mod pack;
mod rounds;
mod sshkey;
