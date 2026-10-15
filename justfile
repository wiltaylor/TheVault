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

# Build the Linux desktop app (debug, no bundles) into apps/desktop/src-tauri/target/debug/thevault
[group('desktop')]
desktop-build: (require-tool "cargo-tauri") web-build
	cd apps/desktop && cargo tauri build --debug --no-bundle

# Run the desktop app's unit tests, then the S6 end-to-end tests with tauri-driver under Xvfb
[group('desktop')]
desktop-test: (require-tool "tauri-driver") (require-tool "xvfb-run") (require-tool "gnome-keyring-daemon") desktop-build
	cd apps/desktop/src-tauri && cargo test && cargo build --example fake-notifyd
	cargo build -p server -p testkit --bins
	xvfb-run -a pnpm -C apps/desktop/e2e test

# Build the release bundles, AppImage, deb and rpm, into target/desktop-release with their SHA256SUMS (D11)
[group('desktop')]
desktop-release: (require-tool "cargo-tauri") web-build
	rm -rf apps/desktop/src-tauri/target/release/bundle target/desktop-release
	cd apps/desktop && XDG_CACHE_HOME="$PWD/src-tauri/target/tauri-cache" APPIMAGE_EXTRACT_AND_RUN=1 cargo tauri build --bundles appimage,deb,rpm
	mkdir -p target/desktop-release
	cp apps/desktop/src-tauri/target/release/bundle/appimage/*.AppImage apps/desktop/src-tauri/target/release/bundle/deb/*.deb apps/desktop/src-tauri/target/release/bundle/rpm/*.rpm target/desktop-release/
	cd target/desktop-release && sha256sum * > SHA256SUMS

# Build the release bundles, then check that the AppImage starts to the setup page under Xvfb
[group('desktop')]
desktop-release-test: (require-tool "tauri-driver") (require-tool "xvfb-run") (require-tool "gnome-keyring-daemon") desktop-release
	xvfb-run -a node --test --test-timeout=600000 apps/desktop/e2e/appimage.smoke.mjs

# Check that packaging/aur/.SRCINFO matches the PKGBUILD, as makepkg prints it
[group('desktop')]
aur-check: (require-tool "makepkg")
	cd packaging/aur && makepkg --printsrcinfo | diff -u .SRCINFO -

# Write the AUR thevault-bin PKGBUILD and .SRCINFO for the deb in target/desktop-release into target/desktop-release/thevault-bin-aur.tar.gz
[group('desktop')]
aur-release: (require-tool "makepkg") (require-tool "jq")
	#!/usr/bin/env bash
	set -euo pipefail
	version=$(jq -r .version apps/desktop/src-tauri/tauri.conf.json)
	deb="target/desktop-release/TheVault_${version}_amd64.deb"
	test -f "$deb" || { echo "$deb is missing; run just desktop-release" >&2; exit 1; }
	sum=$(sha256sum "$deb" | cut -d' ' -f1)
	out=target/desktop-release/aur/thevault-bin
	rm -rf target/desktop-release/aur && mkdir -p "$out"
	sed -e "s/^pkgver=.*/pkgver=${version}/" -e "s/^sha256sums=.*/sha256sums=('${sum}')/" packaging/aur/PKGBUILD > "$out/PKGBUILD"
	(cd "$out" && makepkg --printsrcinfo > .SRCINFO)
	tar -czf target/desktop-release/thevault-bin-aur.tar.gz -C target/desktop-release/aur thevault-bin
	rm -rf target/desktop-release/aur

# Build the Android debug APK for aarch64 into apps/android/src-tauri/gen/android/app/build/outputs/apk
[group('android')]
android-build: (require-tool "cargo-tauri") (require-tool "cargo-ndk") web-build
	cd apps/android && cargo tauri android build --debug --apk --target aarch64

# Build the signed release APK for aarch64 into target/android-release with its SHA256SUMS, signed with the keystore THEVAULT_ANDROID_KEYSTORE names (D10)
[group('android')]
android-release: (require-tool "cargo-tauri") (require-tool "cargo-ndk") (require-tool "jq") web-build
	#!/usr/bin/env bash
	set -euo pipefail
	for var in THEVAULT_ANDROID_KEYSTORE THEVAULT_ANDROID_KEYSTORE_PASSWORD THEVAULT_ANDROID_KEY_ALIAS; do
		if [ -z "${!var:-}" ]; then
			echo "$var is not set; see docs/android-release.md" >&2
			exit 1
		fi
	done
	test -f "$THEVAULT_ANDROID_KEYSTORE" || { echo "keystore $THEVAULT_ANDROID_KEYSTORE does not exist" >&2; exit 1; }
	apksigner=$(ls -d "$ANDROID_HOME"/build-tools/*/apksigner | sort -V | tail -n 1)
	version=$(jq -r .version apps/android/src-tauri/tauri.conf.json)
	apks=apps/android/src-tauri/gen/android/app/build/outputs/apk
	rm -rf "$apks" target/android-release
	# Every container shares the Gradle home and the host network, so a Gradle client could hand the
	# build to another container's idle daemon, which cannot see this checkout. Build without one.
	(cd apps/android && GRADLE_OPTS="${GRADLE_OPTS:-} -Dorg.gradle.daemon=false" cargo tauri android build --apk --target aarch64)
	mkdir -p target/android-release
	apk="target/android-release/TheVault_${version}_aarch64.apk"
	built=$(find "$apks" -name '*-release.apk' | head -n 1)
	test -n "$built" || { echo "no signed release APK under $apks" >&2; exit 1; }
	cp "$built" "$apk"
	"$apksigner" verify --min-sdk-version 33 --print-certs "$apk"
	(cd target/android-release && sha256sum -- *.apk > SHA256SUMS)
	echo "wrote $apk"

# Run the app's Rust tests (the core against a vault-server), then the S5 Kotlin unit tests with Robolectric on the JVM against a host build of the plugin's Rust library
[group('android')]
android-test: (require-tool "java") android-build
	cargo build -p server --bin vault-server
	cd apps/android/src-tauri && VAULT_SERVER_BIN="$PWD/../../../target/debug/vault-server" cargo test -p tauri-plugin-vault-device -p thevault-android
	cd apps/android/src-tauri && cargo rustc -p tauri-plugin-vault-device --lib --crate-type cdylib
	cd apps/android/src-tauri/gen/android && VAULT_CORE_LIBRARY="$PWD/../../target/debug/libtauri_plugin_vault_device.so" ./gradlew --no-daemon testDebugUnitTest
