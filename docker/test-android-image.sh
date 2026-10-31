#!/usr/bin/env bash
# End-to-end checks of the Android box image (docker/android.Dockerfile), run on a host with
# Docker and /dev/kvm:
#
#   docker/test-android-image.sh
#
# Builds the browser box and the Android box from this checkout, or checks IMAGE=<ref> when given,
# then checks:
#
# - it refuses to start without /dev/kvm, and says how to pass it;
# - with /dev/kvm and a token it comes up healthy, and adb, emulator and avdmanager are on PATH
#   for commands, beside playwright-cli;
# - the emulator boots, and Google Play (com.android.vending) and Google Play services
#   (com.google.android.gms) are installed;
# - the emulator window is on the desktop;
# - adb drives it: a key event, a UI dump, and a screenshot that /file reads back as a PNG.
#
# Every port is one Docker picks on 127.0.0.1; the container, and the images when it built them,
# are removed when it ends. KEEP_IMAGE=1 keeps built images.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ ! -e /dev/kvm ]; then
  echo "this host has no /dev/kvm; the Android emulator needs it" >&2
  exit 1
fi

run="thevault-androidtest-$(date +%s)-$$"
token="tok-$run-0123456789abcdef"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/$run.XXXXXX")
failures=0
built=

cleanup() {
  docker ps -aq --filter "label=$run" | xargs -r docker rm -f -v > /dev/null
  if [ -n "$built" ] && [ "${KEEP_IMAGE:-}" != 1 ]; then
    docker rmi -f "$image" "$browser_image" > /dev/null 2>&1 || true
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

# Retry "$@" every second for up to $1 seconds.
retry() {
  local secs=$1 i
  shift
  for i in $(seq 1 "$secs"); do
    if "$@" > /dev/null 2>&1; then return 0; fi
    sleep 1
  done
  return 1
}

browser_image="$run-browser:test"
if [ -n "${IMAGE:-}" ]; then
  image=$IMAGE
else
  image="$run:test"
  built=1
  echo "building $browser_image"
  docker build -f docker/browser.Dockerfile --platform linux/amd64 -t "$browser_image" . > "$tmp/build.log" 2>&1 ||
    {
      tail -n 40 "$tmp/build.log"
      exit 1
    }
  echo "building $image"
  docker build -f docker/android.Dockerfile --platform linux/amd64 \
    --build-arg BROWSER_IMAGE="$browser_image" -t "$image" . > "$tmp/build.log" 2>&1 ||
    {
      tail -n 40 "$tmp/build.log"
      exit 1
    }
fi

# 1. No /dev/kvm, no start.
out=$(docker run --rm --label "$run" -e BROWSER_TOKEN="$token" "$image" 2>&1) && status=0 || status=$?
check "it refuses to start without /dev/kvm (exit $status)" test "$status" -ne 0
check "and says how to pass it" grep -qF -- '--device /dev/kvm' <<< "$out"

# 2. With /dev/kvm and a token.
docker run -d --name "$run" --label "$run" --shm-size 1g -e BROWSER_TOKEN="$token" \
  --device /dev/kvm --group-add "$(stat -c %g /dev/kvm)" \
  -p 127.0.0.1::7700 -p 127.0.0.1::6080 "$image" > /dev/null
port=$(docker port "$run" 7700/tcp | head -n 1 | sed 's/.*://')
base="http://127.0.0.1:$port"
check "/health answers on a free port ($port)" retry 60 curl -fsS "$base/health"
check "the container becomes healthy" retry 60 sh -c "docker inspect -f '{{.State.Health.Status}}' '$run' | grep -qx healthy"

# POST /run $1 with timeout $2 (default 60); prints the JSON answer.
run_cmd() {
  curl -sS -H "Authorization: Bearer $token" -H 'content-type: application/json' \
    --data "$(jq -cn --arg c "$1" --argjson t "${2:-60}" '{command: $c, timeout_s: $t}')" "$base/run"
}
get() {
  local path=$1
  shift
  curl -sS -H "Authorization: Bearer $token" "$@" "$base$path"
}
booted() {
  run_cmd 'adb wait-for-device && adb shell getprop sys.boot_completed' 30 |
    jq -e '.stdout | rtrimstr("\n") | rtrimstr("\r") == "1"'
}

r=$(run_cmd 'command -v adb emulator avdmanager playwright-cli')
check "adb, emulator, avdmanager and playwright-cli are on PATH ($(jq -c .stdout <<< "$r"))" \
  jqe '.exit == 0 and (.stdout | split("\n") | length) == 5' <<< "$r"

# 3. The emulator boots with Google Play.
started=$(date +%s)
check "the emulator boots" retry 300 booted
echo "boot took $(($(date +%s) - started))s"
r=$(run_cmd 'adb shell pm path com.android.vending')
check "Google Play is installed ($(jq -c .stdout <<< "$r"))" jqe '.stdout | startswith("package:")' <<< "$r"
r=$(run_cmd 'adb shell pm path com.google.android.gms')
check "Google Play services are installed" jqe '.stdout | startswith("package:")' <<< "$r"

# 4. The window is on the desktop.
r=$(run_cmd 'xwininfo -root -tree | grep -ci "android emulator"')
check "the emulator window is on the desktop" jqe '(.stdout | rtrimstr("\n") | tonumber) > 0' <<< "$r"

# 5. adb drives it.
r=$(run_cmd 'adb shell input keyevent KEYCODE_HOME')
check "adb sends a key event" jqe '.exit == 0' <<< "$r"
r=$(run_cmd 'adb exec-out uiautomator dump /dev/tty' 60)
check "uiautomator dumps the screen" jqe '.stdout | contains("<hierarchy")' <<< "$r"
r=$(run_cmd 'adb exec-out screencap -p > /work/android.png')
check "adb takes a screenshot" jqe '.exit == 0' <<< "$r"
get '/file?path=/work/android.png' -o "$tmp/shot.png" -D "$tmp/shot.headers" > /dev/null || true
check "/file answers it as image/png" grep -qi '^content-type: image/png' "$tmp/shot.headers"
check "and the bytes are a PNG" sh -c "head -c 8 '$tmp/shot.png' | od -An -tx1 | tr -d ' \n' | grep -qx 89504e470d0a1a0a"

size=$(docker image inspect -f '{{.Size}}' "$image")
echo "image $image: $((size / 1024 / 1024)) MiB"

if [ "$failures" -ne 0 ]; then
  docker logs "$run" 2>&1 | tail -n 60
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed"
