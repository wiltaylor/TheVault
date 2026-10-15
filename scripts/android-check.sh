#!/usr/bin/env bash
# Lint the Android app's server core for target_os = "android" (ticket 76).
#
# Host builds never take Android's cfg paths, and std APIs such as File::lock compile for Android
# but answer Unsupported at run time. This runs clippy, with clippy.toml's ban on std's file locks,
# over the crates the app embeds, built for the x86_64 emulator target. Run it through ./dev:
#
#     ./dev scripts/android-check.sh
set -euo pipefail
cd "$(dirname "$0")/.."
cargo ndk -t x86_64 clippy -p store -p embedded -- -D warnings
