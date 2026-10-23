#!/bin/sh
# Start the XFCE session on the VNC server's display once it is up, with its own session bus,
# and keep the screen from blanking: the user watches it rather than works at it.
set -eu

display=${DISPLAY:-:1}
socket="/tmp/.X11-unix/X${display#:}"
i=0
while [ ! -S "$socket" ]; do
  i=$((i + 1))
  if [ "$i" -gt 100 ]; then
    echo "thevault-desktop: display $display did not come up" >&2
    exit 1
  fi
  sleep 0.1
done
xset s off -dpms 2> /dev/null || true
exec dbus-launch --exit-with-session startxfce4
