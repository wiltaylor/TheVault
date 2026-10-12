//! `vault`: the command-line client of the TheVault HTTP API.

use clap::Parser;

#[derive(Parser)]
#[command(name = "vault", version, about = "Command-line client for TheVault")]
struct Args {}

fn main() {
    Args::parse();
}
