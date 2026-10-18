# TheVault tasks. Run them through the dev container: ./dev just <recipe>

import '.just/shared.just'

# Verification gate
mod ci '.just/ci'

alias server := server-run
alias test-e2e := web-e2e-test

# The workspace version, which every crate and the vault CLI carry
version := `sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1`

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
	cargo build -p server -p testkit -p cli --bins
	xvfb-run -a pnpm -C apps/desktop/e2e test

# Build the release bundles, AppImage, deb and rpm, into target/desktop-release with their SHA256SUMS (D11); with the updater key (TAURI_SIGNING_PRIVATE_KEY, docs/desktop.md) the AppImage gets its updater signature, .AppImage.sig
[group('desktop')]
desktop-release: (require-tool "cargo-tauri") (require-tool "jq") web-build
	#!/usr/bin/env bash
	set -euo pipefail
	updater_env="${XDG_CONFIG_HOME:-$HOME/.config}/thevault/updater/updater.env"
	if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -f "$updater_env" ]; then . "$updater_env"; fi
	rm -rf apps/desktop/src-tauri/target/release/bundle target/desktop-release
	# The updater signature is made below, bound to the version, so the build makes none itself.
	(cd apps/desktop && env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD XDG_CACHE_HOME="$PWD/src-tauri/target/tauri-cache" APPIMAGE_EXTRACT_AND_RUN=1 cargo tauri build --bundles appimage,deb,rpm)
	mkdir -p target/desktop-release
	cp apps/desktop/src-tauri/target/release/bundle/appimage/*.AppImage apps/desktop/src-tauri/target/release/bundle/deb/*.deb apps/desktop/src-tauri/target/release/bundle/rpm/*.rpm target/desktop-release/
	if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
		version=$(jq -r .version apps/desktop/src-tauri/tauri.conf.json)
		key=(-k "$TAURI_SIGNING_PRIVATE_KEY")
		if [ -f "$TAURI_SIGNING_PRIVATE_KEY" ]; then key=(-f "$TAURI_SIGNING_PRIVATE_KEY"); fi
		for image in target/desktop-release/*.AppImage; do
			env -u TAURI_SIGNING_PRIVATE_KEY cargo tauri signer sign "${key[@]}" --app-version "$version" "$image" > /dev/null
			echo "signed $image for the updater"
		done
	else
		echo "TAURI_SIGNING_PRIVATE_KEY is not set: the AppImage has no updater signature, so installed apps will not update to it (docs/desktop.md)" >&2
	fi
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

# Build the Android debug APK, for aarch64 or x86_64 (the emulator), into apps/android/src-tauri/gen/android/app/build/outputs/apk
[group('android')]
android-build target='aarch64': (require-tool "cargo-tauri") (require-tool "cargo-ndk") web-build
	# Debug symbols make the APK about 500 MB; leave them out unless CARGO_PROFILE_DEV_DEBUG/STRIP say otherwise.
	cd apps/android && CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}" CARGO_PROFILE_DEV_STRIP="${CARGO_PROFILE_DEV_STRIP:-symbols}" GRADLE_OPTS="${GRADLE_OPTS:-} -Dorg.gradle.daemon=false" cargo tauri android build --debug --apk --target {{target}}

# Run docs/android-manual-checks.md on a headless Android 15 emulator, and write the results to .scratch/emulator/RESULTS.generated.md; THEVAULT_APK tests another APK, a release APK too (its package id comes from the APK)
[group('android')]
android-emulator-checks: (require-tool "emulator") (require-tool "adb") (require-tool "cargo")
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -z "${THEVAULT_APK:-}" ]; then just android-build x86_64; else just web-build; fi
	cargo build -p server -p cli --bins
	# A factory-fresh data partition each run; -writable-system lets the run swap Google Messages for its test stand-in.
	apps/android/emulator/boot.sh --wipe -writable-system
	status=0
	node apps/android/emulator/checks.mjs || status=$?
	adb emu kill >/dev/null 2>&1 || true
	exit $status

# Lint the crates the Android app embeds for target_os = "android" (ticket 76)
[group('android')]
android-check: (require-tool "cargo-ndk")
	scripts/android-check.sh

# Build the signed release APK for aarch64 into target/android-release with its SHA256SUMS, signed with the keystore THEVAULT_ANDROID_KEYSTORE names, read from ~/.config/thevault/android/keystore.env when it is not set (D10)
[group('android')]
android-release: (require-tool "cargo-tauri") (require-tool "cargo-ndk") (require-tool "jq") web-build
	#!/usr/bin/env bash
	set -euo pipefail
	keystore_env="${XDG_CONFIG_HOME:-$HOME/.config}/thevault/android/keystore.env"
	if [ -z "${THEVAULT_ANDROID_KEYSTORE:-}" ] && [ -f "$keystore_env" ]; then . "$keystore_env"; fi
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

# Build the release APK for x86_64 and run it on the headless emulator: it must start, stay up, onboard and pair, and pass checks 1, 2, 3, 7, 8, 11, 17 and 29; builds the aarch64 release too when the release keystore is there, and records both APKs' sha256 in target/android-release/SMOKE_PASSED, which publish-clients requires (docs/android-release.md)
[group('android')]
android-release-smoke: (require-tool "cargo-tauri") (require-tool "cargo-ndk") (require-tool "emulator") (require-tool "adb") (require-tool "jq") (require-tool "cargo") web-build
	#!/usr/bin/env bash
	set -euo pipefail
	keystore_env="${XDG_CONFIG_HOME:-$HOME/.config}/thevault/android/keystore.env"
	if [ -z "${THEVAULT_ANDROID_KEYSTORE:-}" ] && [ -f "$keystore_env" ]; then . "$keystore_env"; fi
	build_tools=$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -n 1)
	version=$(jq -r .version apps/android/src-tauri/tauri.conf.json)
	apks=apps/android/src-tauri/gen/android/app/build/outputs/apk
	smoke=target/android-release-smoke
	rm -rf "$apks" "$smoke" target/android-release/SMOKE_PASSED
	mkdir -p "$smoke"
	# The release build type, as android-release builds it, for the emulator's ABI. Gradle signs it
	# with the release key when THEVAULT_ANDROID_KEYSTORE is set and leaves it unsigned otherwise;
	# an unsigned one is signed here with the debug key, so the test needs no release keystore.
	(cd apps/android && GRADLE_OPTS="${GRADLE_OPTS:-} -Dorg.gradle.daemon=false" cargo tauri android build --apk --target x86_64)
	built=$(find "$apks" -name '*-release.apk' -o -name '*-release-unsigned.apk' | head -n 1)
	test -n "$built" || { echo "no release APK under $apks" >&2; exit 1; }
	apk="$smoke/TheVault_${version}_x86_64.apk"
	case "$built" in
	*-unsigned.apk)
		debug_ks="${ANDROID_USER_HOME:-$HOME/.android}/debug.keystore"
		if [ ! -f "$debug_ks" ]; then
			mkdir -p "$(dirname "$debug_ks")"
			keytool -genkeypair -keystore "$debug_ks" -storepass android -keypass android -alias androiddebugkey \
				-keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US" > /dev/null
		fi
		"$build_tools/apksigner" sign --ks "$debug_ks" --ks-pass pass:android --ks-key-alias androiddebugkey --key-pass pass:android --out "$apk" "$built"
		echo "signed $apk with the debug key: there is no release keystore here"
		;;
	*) cp "$built" "$apk" ;;
	esac
	"$build_tools/apksigner" verify --min-sdk-version 33 "$apk"
	# The phone's APK, from the same tree, so the result below vouches for what is published.
	released=""
	if [ -n "${THEVAULT_ANDROID_KEYSTORE:-}" ] && [ -f "$THEVAULT_ANDROID_KEYSTORE" ]; then
		just android-release
		released=yes
	else
		echo "no release keystore: testing the x86_64 build alone; no aarch64 APK is vouched for" >&2
	fi
	cargo build -p server -p cli --bins
	apps/android/emulator/boot.sh --wipe -writable-system
	out=.scratch/emulator-release
	status=0
	THEVAULT_APK="$apk" THEVAULT_SMOKE=1 THEVAULT_CHECKS=1,2,3,7,8,11,17,29 THEVAULT_CHECKS_ONLY=1 THEVAULT_EMULATOR_OUT="$out" \
		node apps/android/emulator/checks.mjs || status=$?
	adb emu kill >/dev/null 2>&1 || true
	if [ "$status" -ne 0 ] || ! jq -e '.status == "pass"' "$out/smoke.json" > /dev/null 2>&1; then
		echo "android-release-smoke FAILED for $apk; see $out/RESULTS.generated.md and $out/smoke.json. Nothing is recorded, so publish-clients will not publish this release." >&2
		exit 1
	fi
	mkdir -p target/android-release
	{
		echo "# just android-release-smoke passed $(date -u +%Y-%m-%dT%H:%M:%SZ): $apk started, paired and passed checks 1, 2, 3, 7, 8, 11, 17, 29."
		echo "# publish-clients publishes a release APK only when its sha256 is listed here."
		(cd "$smoke" && sha256sum -- *.apk)
		if [ -n "$released" ]; then (cd target/android-release && sha256sum -- *.apk); fi
	} > target/android-release/SMOKE_PASSED
	cat target/android-release/SMOKE_PASSED

# Run the app's Rust tests (the core against a vault-server), then the S5 Kotlin unit tests with Robolectric on the JVM against a host build of the plugin's Rust library
[group('android')]
android-test: (require-tool "java") android-build
	cargo build -p server --bin vault-server
	cd apps/android/src-tauri && VAULT_SERVER_BIN="$PWD/../../../target/debug/vault-server" cargo test -p tauri-plugin-vault-device -p thevault-android
	cd apps/android/src-tauri && cargo rustc -p tauri-plugin-vault-device --lib --crate-type cdylib
	cd apps/android/src-tauri/gen/android && VAULT_CORE_LIBRARY="$PWD/../../target/debug/libtauri_plugin_vault_device.so" ./gradlew --no-daemon testDebugUnitTest

# Build the static vault CLI for both Linux architectures into target/cli-release, as vault-<version>-linux-<arch>, with their SHA256SUMS (D27)
[group('cli')]
cli-release: (cli-release-build "x86_64") (cli-release-build "aarch64")
	cd target/cli-release && sha256sum -- vault-{{version}}-linux-* > SHA256SUMS
	cat target/cli-release/SHA256SUMS

# Build the static vault CLI for one architecture, x86_64 or aarch64, against musl with zig as the linker, into target/cli-release/vault-<version>-linux-<arch>
[group('cli')]
cli-release-build arch: (require-tool "cargo-zigbuild") (require-tool "zig")
	cargo zigbuild -q --profile cli -p cli --bin vault --target {{arch}}-unknown-linux-musl
	mkdir -p target/cli-release
	rm -f target/cli-release/vault-*-linux-{{arch}}
	cp target/{{arch}}-unknown-linux-musl/cli/vault target/cli-release/vault-{{version}}-linux-{{arch}}
	file target/cli-release/vault-{{version}}-linux-{{arch}}

# Copy the newest app builds and static vault CLI builds into a server's clients folder (its --clients, by default <vault>/.vault/clients) and rewrite its manifest.json, which a running server announces to the apps so they update themselves; build='missing' builds what is not built yet, build='all' builds everything again, build='none' publishes only what is there; a release APK is published only when android-release-smoke passed it, unless smoke='skip' (emergencies only)
[group('clients')]
publish-clients dir build='missing' smoke='required': (require-tool "cargo") (require-tool "jq")
	#!/usr/bin/env bash
	set -euo pipefail
	case "{{build}}" in missing|all|none) ;; *) echo "build must be missing, all or none, not {{build}}" >&2; exit 1 ;; esac
	case "{{smoke}}" in required|skip) ;; *) echo "smoke must be required or skip, not {{smoke}}" >&2; exit 1 ;; esac
	# The signing keys outside the repository (docs/android-release.md, docs/desktop.md); ./dev reads them too.
	config="${XDG_CONFIG_HOME:-$HOME/.config}/thevault"
	if [ -z "${THEVAULT_ANDROID_KEYSTORE:-}" ] && [ -f "$config/android/keystore.env" ]; then . "$config/android/keystore.env"; fi
	if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -f "$config/updater/updater.env" ]; then . "$config/updater/updater.env"; fi
	cargo build -q -p cli --bin vault
	vault=target/debug/vault
	dir="{{dir}}"
	mkdir -p "$dir"
	shopt -s nullglob
	newest() { if [ $# -gt 0 ]; then ls -t -- "$@" | head -n 1; fi; }
	# Android: the signed release APK, built when the release keystore is there, or else the debug
	# APK of the last phone build, labelled debug.
	apks=apps/android/src-tauri/gen/android/app/build/outputs/apk
	debug_apk() { newest "$apks"/arm64/debug/*-debug.apk "$apks"/universal/debug/*-debug.apk; }
	keystore=""
	if [ -n "${THEVAULT_ANDROID_KEYSTORE:-}" ] && [ -f "$THEVAULT_ANDROID_KEYSTORE" ]; then keystore=yes; fi
	android_toolchain=""
	if command -v cargo-ndk > /dev/null 2>&1; then android_toolchain=yes; fi
	# A release APK is published only when android-release-smoke listed its sha256 in SMOKE_PASSED:
	# release 0.1.1 crashed at start on the phone while every test, all on the debug build, passed.
	refused=""
	release_apk=$(newest target/android-release/*.apk)
	if [ -n "$keystore" ] && [ -n "$android_toolchain" ] && { [ "{{build}}" = all ] || { [ -z "$release_apk" ] && [ "{{build}}" = missing ]; }; }; then
		if [ "{{smoke}}" = skip ]; then
			just android-release
		elif ! just android-release-smoke; then
			echo "android-release-smoke failed, so the Android release is not published" >&2
			refused=yes
		fi
		release_apk=$(newest target/android-release/*.apk)
	fi
	if [ -n "$release_apk" ] && [ -z "$refused" ]; then
		sum=$(sha256sum "$release_apk" | cut -d' ' -f1)
		if [ "{{smoke}}" = skip ]; then
			echo "WARNING: publishing $release_apk without the release smoke test (smoke=skip)" >&2
		elif ! grep -q "^$sum " target/android-release/SMOKE_PASSED 2> /dev/null; then
			echo "refusing to publish $release_apk: its sha256 $sum is not in target/android-release/SMOKE_PASSED, so it has not passed the release smoke test. Run ./dev just android-release-smoke, which builds and tests the release, then publish again; in an emergency only, pass smoke=skip: just publish-clients $dir {{build}} skip" >&2
			refused=yes
		fi
	fi
	if [ -n "$refused" ]; then
		echo "not publishing an Android build; the desktop builds are still published" >&2
	else
		debug=$(debug_apk)
		if [ -z "$release_apk" ] && { [ "{{build}}" = all ] || { [ -z "$debug" ] && [ "{{build}}" = missing ]; }; }; then
			if [ -n "$android_toolchain" ]; then
				just android-build aarch64
				debug=$(debug_apk)
			else
				echo "no APK is built and there is no Android toolchain here; skipping Android" >&2
			fi
		fi
		if [ -n "$release_apk" ]; then
			"$vault" clients publish --dir "$dir" --build release "$release_apk" > /dev/null
			echo "published $release_apk"
		elif [ -n "$debug" ]; then
			version=$(jq -r .version apps/android/src-tauri/tauri.conf.json)
			"$vault" clients publish --dir "$dir" --build debug --version "$version" "$debug" > /dev/null
			echo "published $debug as a debug build: there is no signed release APK (just android-release-smoke)"
		fi
	fi
	# Linux: the AppImage, with its updater signature beside it when there is one, deb and rpm of
	# the last desktop release build.
	desktop=(target/desktop-release/*.AppImage target/desktop-release/*.deb target/desktop-release/*.rpm)
	if [ "{{build}}" = all ] || { [ ${#desktop[@]} -eq 0 ] && [ "{{build}}" = missing ]; }; then
		just desktop-release
		desktop=(target/desktop-release/*.AppImage target/desktop-release/*.deb target/desktop-release/*.rpm)
	fi
	if [ ${#desktop[@]} -gt 0 ]; then
		"$vault" clients publish --dir "$dir" --build release "${desktop[@]}" > /dev/null
		printf 'published %s\n' "${desktop[@]}"
	fi
	# The static vault CLI for both architectures (platform cli), which agents download from the
	# server at /api/clients/cli/linux-<arch> (D27).
	cli=(target/cli-release/vault-*-linux-x86_64 target/cli-release/vault-*-linux-aarch64)
	if [ "{{build}}" = all ] || { [ ${#cli[@]} -lt 2 ] && [ "{{build}}" = missing ]; }; then
		just cli-release
		cli=(target/cli-release/vault-*-linux-x86_64 target/cli-release/vault-*-linux-aarch64)
	fi
	if [ ${#cli[@]} -gt 0 ]; then
		"$vault" clients publish --dir "$dir" --build release "${cli[@]}" > /dev/null
		printf 'published %s\n' "${cli[@]}"
	fi
	if [ -f "$dir/manifest.json" ]; then
		jq -r '.clients[] | "\(.platform)\t\(.kind)\t\(.arch)\t\(.version)\t\(.build)\t\(.size)\t\(.filename)\t\(if .signature then "signed" else "" end)"' "$dir/manifest.json"
	else
		echo "nothing is published in $dir"
	fi
	if [ -n "$refused" ]; then exit 1; fi

# Build the server image (docker/Dockerfile) for linux/amd64 without app downloads; runs on the host, not through ./dev
[group('docker')]
image-build tag='thevault:local': (require-host-tool "docker")
	docker build -f docker/Dockerfile --platform linux/amd64 -t {{tag}} .

# Build the server image with this version's APK and AppImage baked in, from dir, a clients folder publish-clients wrote (read only); runs on the host
[group('docker')]
image-clients-build dir tag='thevault:local': (require-host-tool "docker") (require-host-tool "jq")
	#!/usr/bin/env bash
	set -euo pipefail
	version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' Cargo.toml)
	stage=$(mktemp -d)
	trap 'rm -rf "$stage"' EXIT
	docker/stage-clients.sh "{{dir}}" "$stage" "$version"
	docker build -f docker/Dockerfile --platform linux/amd64 --build-context "clients=$stage" -t {{tag}} .

# Build and check the release image exactly as .github/workflows/image.yml does, from assets, a folder of the release's APK, AppImage and SHA256SUMS files (read only), tagging ghcr.io/wiltaylor/thevault (or image) <version>, <major>.<minor> and latest locally without pushing; runs on the host
[group('docker')]
image-release assets image='ghcr.io/wiltaylor/thevault': (require-host-tool "docker") (require-host-tool "jq") (require-host-tool "curl")
	IMAGE={{image}} docker/release-image.sh "{{assets}}"

# Build the image and check it end to end in throwaway containers on free ports: first-run refusal and init, login, version, healthcheck, uid, upgrade backups, an outside vault, and with clients (a publish-clients folder) the downloads; runs on the host
[group('docker')]
image-test clients='': (require-host-tool "docker") (require-host-tool "jq") (require-host-tool "curl") (require-host-tool "python3")
	docker/test-image.sh "{{clients}}"

# Check deploy/compose.yaml end to end on an image built from this checkout, in a throwaway compose project on free ports: config of each profile, the default profile's login, password change, backup and restore, the https profile on Caddy's internal CA, and cloudflared refusing a dummy token; runs on the host
[group('docker')]
compose-test: (require-host-tool "docker") (require-host-tool "jq") (require-host-tool "curl")
	docker/test-compose.sh
