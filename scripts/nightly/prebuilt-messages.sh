#!/usr/bin/env bash
# Device bundle only: the real builder ran on dev. Android has no javac.
set -euo pipefail
test -s target/android-emulator/fake-messages.apk
