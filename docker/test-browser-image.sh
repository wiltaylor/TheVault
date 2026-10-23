#!/usr/bin/env bash
# End-to-end checks of the browser box image (docker/browser.Dockerfile, browser B1, B2, B9), run
# on a host with Docker:
#
#   docker/test-browser-image.sh
#
# Builds the image from this checkout, or checks IMAGE=<ref> when given (docker/release-browser-
# image.sh does that before it pushes), then checks:
#
# - it refuses to start without BROWSER_TOKEN;
# - with a token it comes up healthy, /health reports this version, display :1 and the pinned
#   playwright-cli, and /run without the token is 401;
# - commands run as agent (uid 1000) in /work with HOME /home/agent, and `playwright-cli --help`
#   works;
# - the desktop is up: the XFCE session on display :1, noVNC serving vnc.html on 6080, and the
#   terminal's and desktop's monospace font is JetBrainsMonoNL Nerd Font Mono, as xfconf reads it
#   in the running session;
# - `playwright-cli open about:blank --headed && playwright-cli screenshot` puts a Chromium window
#   on the desktop, and /files and /file read the screenshot back as a PNG, and a headless session
#   opens as well;
# - a run past its timeout is killed;
# - the exec service is a static binary.
#
# Every port is one Docker picks on 127.0.0.1; the container, and the image when it built one,
# are removed when it ends. KEEP_IMAGE=1 keeps a built image.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

run="thevault-browsertest-$(date +%s)-$$"
token="tok-$run-0123456789abcdef"
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' Cargo.toml)
pinned=$(sed -n 's/^ARG PLAYWRIGHT_CLI_VERSION=//p' docker/browser.Dockerfile)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/$run.XXXXXX")
failures=0
built=

cleanup() {
  docker ps -aq --filter "label=$run" | xargs -r docker rm -f -v > /dev/null
  if [ -n "$built" ] && [ "${KEEP_IMAGE:-}" != 1 ]; then
    docker rmi -f "$image" > /dev/null 2>&1 || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

pass() { printf 'PASS %s\n' "$*"; }
fail() {
  printf 'FAIL %s\n' "$*"
  failures=$((failures + 1))
}
check() {
  local what=$1
  shift
  if "$@"; then pass "$what"; else fail "$what"; fi
}
jqe() { jq -e "$@" > /dev/null; }

# Retry "$@" every half second for up to $1 seconds.
retry() {
  local secs=$1 i
  shift
  for i in $(seq 1 $((secs * 2))); do
    if "$@" > /dev/null 2>&1; then return 0; fi
    sleep 0.5
  done
  return 1
}

if [ -n "${IMAGE:-}" ]; then
  image=$IMAGE
else
  image="$run:test"
  echo "building $image"
  built=1
  docker build -f docker/browser.Dockerfile --platform linux/amd64 -t "$image" . > "$tmp/build.log" 2>&1 ||
    {
      tail -n 40 "$tmp/build.log"
      exit 1
    }
fi

# 1. No token, no start.
out=$(docker run --rm --label "$run" "$image" 2>&1) && status=0 || status=$?
check "it refuses to start without BROWSER_TOKEN (exit $status)" test "$status" -ne 0
check "and says how to make one" grep -qF 'openssl rand -hex 32' <<< "$out"

# 2. With a token.
docker run -d --name "$run" --label "$run" --shm-size 1g -e BROWSER_TOKEN="$token" \
  -p 127.0.0.1::7700 -p 127.0.0.1::6080 "$image" > /dev/null
port=$(docker port "$run" 7700/tcp | head -n 1 | sed 's/.*://')
vnc=$(docker port "$run" 6080/tcp | head -n 1 | sed 's/.*://')
base="http://127.0.0.1:$port"
check "/health answers on a free port ($port)" retry 60 curl -fsS "$base/health"
health=$(curl -fsS "$base/health" || echo '{}')
check "/health is ok ($health)" jqe '.ok == true' <<< "$health"
check "/health reports version $version" jqe --arg v "$version" '.version == $v' <<< "$health"
check "/health reports display :1" jqe '.display == ":1"' <<< "$health"
check "/health reports playwright-cli $pinned" jqe --arg v "$pinned" '.playwright_cli == $v' <<< "$health"
check "the container becomes healthy" retry 60 sh -c "docker inspect -f '{{.State.Health.Status}}' '$run' | grep -qx healthy"
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' -d '{"command":"true"}' "$base/run")
check "/run without the token is 401 (got $code)" test "$code" = 401

# POST /run $1 with timeout $2 (default 60); prints the JSON answer.
run_cmd() {
  curl -sS -H "Authorization: Bearer $token" -H 'content-type: application/json' \
    --data "$(jq -cn --arg c "$1" --argjson t "${2:-60}" '{command: $c, timeout_s: $t}')" "$base/run"
}
# GET $1 with the token, extra curl arguments after.
get() {
  local path=$1
  shift
  curl -sS -H "Authorization: Bearer $token" "$@" "$base$path"
}

r=$(run_cmd 'printf "%s|%s|%s|%s\n" "$(id -un)" "$(id -u)" "$PWD" "$HOME"')
check "commands run as agent (uid 1000) in /work with HOME /home/agent ($(jq -c .stdout <<< "$r"))" \
  jqe '.stdout == "agent|1000|/work|/home/agent\n"' <<< "$r"
r=$(run_cmd 'playwright-cli --help')
check "playwright-cli --help works as agent" jqe '.exit == 0 and (.stdout | contains("snapshot"))' <<< "$r"
r=$(run_cmd 'printenv BROWSER_TOKEN || echo unset')
check "commands do not see BROWSER_TOKEN" jqe '.stdout == "unset\n"' <<< "$r"

# 3. The desktop.
check "the XFCE session runs on display :1" retry 60 sh -c \
  "docker exec '$run' sh -c 'pgrep -x xfce4-session && pgrep -x xfwm4 && DISPLAY=:1 xset q'"
code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$vnc/vnc.html")
check "noVNC serves vnc.html on 6080 (got $code)" test "$code" = 200
r=$(run_cmd 'fc-match -f "%{family}" monospace')
check "monospace is JetBrainsMonoNL Nerd Font Mono ($(jq -c .stdout <<< "$r"))" \
  jqe '.stdout | startswith("JetBrainsMonoNL Nerd Font Mono")' <<< "$r"
# xfconf as the running session reads it, on the session's bus.
session_bus='export $(tr "\0" "\n" < /proc/$(pgrep -x xfce4-session)/environ | grep ^DBUS_SESSION_BUS_ADDRESS=)'
r=$(run_cmd "$session_bus; xfconf-query -c xfce4-terminal -p /font-name; xfconf-query -c xfce4-terminal -p /font-use-system; xfconf-query -c xsettings -p /Gtk/MonospaceFontName")
check "xfce4-terminal uses JetBrainsMonoNL Nerd Font Mono, and so does the desktop ($(jq -c .stdout <<< "$r"))" \
  jqe '.stdout == "JetBrainsMonoNL Nerd Font Mono 11\nfalse\nJetBrainsMonoNL Nerd Font Mono 10\n"' <<< "$r"

# 4. playwright-cli drives a headed Chromium, and the screenshot reads back.
r=$(run_cmd 'playwright-cli open about:blank --headed && playwright-cli screenshot' 180)
printf '%s\n' "$r" > "$tmp/open.json"
check "playwright-cli open --headed and screenshot exit 0" jqe '.exit == 0 and .timed_out == false' <<< "$r"
shot=$(jq -r '.stdout' <<< "$r" | grep -o '\.playwright-cli/[^)]*\.png' | head -n 1 || true)
check "the screenshot is named in the output ($shot)" test -n "$shot"
r=$(run_cmd 'xwininfo -root -tree | grep -ci chrom')
check "the browser window is on the desktop" jqe '(.stdout | rtrimstr("\n") | tonumber) > 0' <<< "$r"
listed=$(get '/files?dir=/work/.playwright-cli' || echo '{}')
check "/files lists the screenshot" jqe --arg n "${shot##*/}" '[.entries[]? | select(.kind == "file") | .name] | index($n) != null' <<< "$listed"
get "/file?path=/work/$shot" -o "$tmp/shot.png" -D "$tmp/shot.headers" > /dev/null || true
check "/file answers it as image/png" grep -qi '^content-type: image/png' "$tmp/shot.headers"
check "and the bytes are a PNG" sh -c "head -c 8 '$tmp/shot.png' | od -An -tx1 | tr -d ' \n' | grep -qx 89504e470d0a1a0a"
code=$(get '/file?path=/etc/passwd' -o /dev/null -w '%{http_code}')
check "/file refuses /etc/passwd (got $code)" test "$code" = 403
r=$(run_cmd 'playwright-cli -s=headless open about:blank && playwright-cli -s=headless snapshot' 180)
check "a headless session opens too" jqe '.exit == 0 and .timed_out == false' <<< "$r"
run_cmd 'playwright-cli close-all' > /dev/null || true

# 5. A timeout.
r=$(run_cmd 'sleep 30' 1)
check "a run past its timeout is killed ($(jq -c '{exit, timed_out, ms}' <<< "$r"))" \
  jqe '.timed_out == true and .exit == 137 and .ms < 10000' <<< "$r"

# 6. The exec service is static.
r=$(docker exec "$run" sh -c 'ldd /usr/local/bin/browser-agent 2>&1 || true')
check "browser-agent is statically linked ($r)" grep -qE 'not a dynamic executable|statically linked' <<< "$r"

size=$(docker image inspect -f '{{.Size}}' "$image")
echo "image $image: $((size / 1024 / 1024)) MiB"

if [ "$failures" -ne 0 ]; then
  docker logs "$run" 2>&1 | tail -n 40
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed"
