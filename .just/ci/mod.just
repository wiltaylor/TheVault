set working-directory := '../..'

import '../shared.just'

[default, private]
main:
	@just --list ci

# Everything a change must pass before it can merge, the release scripts' tests too. Playwright runs separately in test-e2e
[group('ci')]
check: rust-fmt-check rust-lint rust-test web-typecheck web-test release-test

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

# Test the release train's scripts: the version calculation, the Android versionCode, how dpkg, rpm and pacman sort the package versions, the image tags and stamping (docs/releasing.md)
[group('ci')]
release-test: (require-tool "git") (require-tool "jq") (require-tool "dpkg") (require-tool "rpm") (require-tool "vercmp")
	scripts/release/test.sh
