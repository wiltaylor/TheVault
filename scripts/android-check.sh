#!/usr/bin/env bash
# Lint the Android app's code for target_os = "android" (tickets 76, 114).
#
# Host builds never take Android's cfg paths, and std APIs such as File::lock compile for Android
# but answer Unsupported at run time. This runs clippy, with clippy.toml's ban on std's file locks,
# for the x86_64 emulator target over the server-workspace crates the app embeds, then over the
# app's own workspace (the app and tauri-plugin-vault-device), whose Tauri context needs web/dist
# (just web-build). Run it through ./dev:
#
#     ./dev scripts/android-check.sh
set -euo pipefail
cd "$(dirname "$0")/.."
cargo ndk -t x86_64 clippy -p store -p embedded -- -D warnings
(cd apps/android/src-tauri && cargo ndk -t x86_64 clippy --workspace --all-targets -- -D warnings)
