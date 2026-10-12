set working-directory := '../..'

import '../shared.just'

[default, private]
main:
	@just --list ci

# Everything a change must pass before it can merge. Playwright runs separately in test-e2e
[group('ci')]
check: rust-fmt-check rust-lint rust-test web-typecheck web-test

# Check Rust formatting without changing files
[group('ci')]
rust-fmt-check: (require-tool "cargo")
	cargo fmt --all --check

# Run clippy over every target with warnings denied
[group('ci')]
rust-lint: (require-tool "cargo")
	cargo clippy --workspace --all-targets -- -D warnings

# Run every Rust test
[group('ci')]
rust-test: (require-tool "cargo")
	cargo test --workspace

# Typecheck the web client
[group('ci')]
web-typecheck: web-deps-install
	pnpm -C web typecheck

# Run the web client's unit tests
[group('ci')]
web-test: web-deps-install
	pnpm -C web test
