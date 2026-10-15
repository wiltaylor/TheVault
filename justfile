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

# Build the Linux desktop app (debug, no bundles). Needs apps/desktop, which lands with ticket 62
[group('desktop')]
desktop-build: (require-path "apps/desktop/src-tauri" "ticket 62") (require-tool "cargo-tauri") web-build
	cd apps/desktop && cargo tauri build --debug --no-bundle

# Run the S6 desktop end-to-end tests with tauri-driver under Xvfb. Needs apps/desktop/e2e, which lands with ticket 62
[group('desktop')]
desktop-test: (require-path "apps/desktop/e2e" "ticket 62") (require-tool "tauri-driver") (require-tool "xvfb-run") desktop-build
	cargo build -p server -p testkit --bins
	xvfb-run -a pnpm -C apps/desktop/e2e test

# Build the Android debug APK for aarch64. Needs apps/android, which lands with ticket 65
[group('android')]
android-build: (require-path "apps/android/src-tauri" "ticket 65") (require-tool "cargo-tauri") (require-tool "cargo-ndk") web-build
	cd apps/android && cargo tauri android build --debug --apk --target aarch64

# Run the S5 Kotlin unit tests with Robolectric on the JVM. Needs the Gradle project, which lands with ticket 65
[group('android')]
android-test: (require-path "apps/android/src-tauri/gen/android/gradlew" "ticket 65") (require-tool "java")
	cd apps/android/src-tauri/gen/android && ./gradlew --no-daemon testDebugUnitTest
