# TheVault tasks. Run them through the dev container: ./dev just <recipe>

import '.just/shared.just'

# Verification gate
mod ci '.just/ci'

alias server := server-run
alias test-e2e := web-e2e-test

[default, private]
main:
	@just --list

# Run the full merge gate: formatting, clippy, Rust tests, web typecheck and unit tests
[group('check')]
check: ci::check

# Format all code
[group('check')]
fmt: rust-fmt

# Format the Rust workspace
[group('rust')]
rust-fmt: (require-tool "cargo")
	cargo fmt --all

# Run vault-server on a local vault folder
[group('rust')]
server-run vault='.scratch/dev-vault' listen='127.0.0.1:8080': (require-tool "cargo")
	mkdir -p {{vault}}
	cargo run -p server --bin vault-server -- --vault {{vault}} --listen {{listen}}

# Run the S6 benchmark: generate a seeded vault, time reindex, query, search, open and save, and write target/bench/report.md
[group('rust')]
bench notes='1000000' seed='42' samples='200': (require-tool "cargo")
	cargo build --release -p bench -p server -p cli --bins
	target/release/vault-bench --notes {{notes}} --seed {{seed}} --samples {{samples}}

# Run the Vite dev server for the web client
[group('web')]
web-dev: web-deps-install
	pnpm -C web dev

# Build the web client into web/dist
[group('web')]
web-build: web-deps-install
	pnpm -C web build

# Run the Playwright tests against vault-server serving the built web client
[group('web')]
web-e2e-test: web-build (require-tool "cargo")
	cargo build -p server -p testkit --bins
	pnpm -C web e2e
