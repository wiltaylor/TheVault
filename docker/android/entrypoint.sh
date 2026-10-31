#!/bin/sh
# The Android box's entrypoint, run under tini as agent: refuse to start without a usable
# /dev/kvm, which the x86_64 emulator needs, then hand over to the browser box's entrypoint.
set -eu

if [ ! -r /dev/kvm ] || [ ! -w /dev/kvm ]; then
  printf 'thevault-android: /dev/kvm is missing or not usable by uid %s: run with --device /dev/kvm --group-add "$(stat -c %%g /dev/kvm)"\n' "$(id -u)" >&2
  exit 1
fi
exec /usr/local/bin/thevault-browser-entrypoint
