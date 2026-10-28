set working-directory := '../..'

import '../shared.just'

[default, private]
main:
	@just --list ci

# Everything a change must pass before it can merge, the release scripts' tests too. Playwright runs separately in test-e2e
[group('ci')]
check: rust-tests-declared rust-fmt-check rust-lint apps-fmt-check apps-lint rust-test web-typecheck web-test release-test

# Check Rust formatting without changing files
[group('ci')]
rust-fmt-check: (require-tool "cargo")
	cargo fmt --all --check

# Run clippy over every target with warnings denied
[group('ci')]
rust-lint: (require-tool "cargo")
	cargo clippy --workspace --all-targets -- -D warnings

# Check the formatting of the crates outside the main workspace: the Android app with tauri-plugin-vault-device, and the desktop app
[group('ci')]
apps-fmt-check: (app-fmt-check "apps/android/src-tauri") (app-fmt-check "apps/desktop/src-tauri")

# Run clippy with warnings denied over the Android and desktop app workspaces on the host, and the Android one for target_os = "android" too (scripts/android-check.sh)
[group('ci')]
apps-lint: (app-lint "apps/android/src-tauri") (app-lint "apps/desktop/src-tauri") (require-tool "cargo-ndk")
	scripts/android-check.sh

[private]
app-fmt-check dir: (require-tool "cargo")
	cd {{dir}} && cargo fmt --all --check

# Each app's Tauri context embeds web/dist, so clippy needs the web client built
[private]
app-lint dir: (require-tool "cargo") web-build
	cd {{dir}} && cargo clippy --workspace --all-targets -- -D warnings

# Fail when a crate's tests/ holds a file its tests/main.rs does not declare: with autotests off, cargo would never build it
[group('ci')]
rust-tests-declared:
	#!/usr/bin/env bash
	set -euo pipefail
	missing=0
	for f in crates/*/tests/*.rs; do
		name=$(basename "$f" .rs)
		[ "$name" = main ] && continue
		main="$(dirname "$f")/main.rs"
		if ! grep -qx "mod $name;" "$main" 2>/dev/null; then
			echo "$f is not declared in $main: add \`mod $name;\`" >&2
			missing=1
		fi
	done
	exit "$missing"

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
