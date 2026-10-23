#!/bin/sh
# The browser box's entrypoint, run under tini as agent (browser B1):
#
# 1. Refuse to start without BROWSER_TOKEN, the bearer token the exec service takes; compose
#    passes it from .env, where `openssl rand -hex 32` makes one.
# 2. Check /home/agent (the browser-home volume, where browser profiles and logins persist) and
#    /work are writable, and give the home the folders the desktop writes to.
# 3. exec supervisord, which starts the VNC server, the desktop, noVNC and the exec service.
#
# BROWSER_GEOMETRY sets the desktop's size, default 1600x900. Every line goes to stderr.
set -eu

log() {
  printf 'thevault-browser: %s\n' "$*" >&2
}

token=${BROWSER_TOKEN:-}
if [ "${#token}" -lt 16 ]; then
  log "BROWSER_TOKEN is not set, or is shorter than 16 characters: set it in .env, made with: openssl rand -hex 32"
  exit 1
fi

for dir in /home/agent /work; do
  if [ ! -w "$dir" ]; then
    log "$dir is not writable by uid $(id -u); give the volume to $(id -u):$(id -g)"
    exit 1
  fi
done
mkdir -p /home/agent/.config /home/agent/.cache /home/agent/Desktop

export HOME=/home/agent
export BROWSER_GEOMETRY="${BROWSER_GEOMETRY:-1600x900}"
exec /usr/bin/supervisord -c /etc/thevault-browser/supervisord.conf
