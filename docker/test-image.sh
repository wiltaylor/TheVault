#!/usr/bin/env bash
# End-to-end checks of the server image (docker/Dockerfile), run on a host with Docker:
#
#   docker/test-image.sh [clients-dir]
#
# Builds the image without clients, and with them when clients-dir (a folder `just
# publish-clients` wrote, only read) is given, then checks the first-run refusal, a first run with
# VAULT_PASSWORD, the backup before an upgrade in semver order, a vault made outside the
# container, and the client downloads. Every container publishes to a port Docker picks on 127.0.0.1, every vault lives in a
# fresh temporary folder, and the containers and images it made are removed when it ends.
# KEEP_IMAGES=1 keeps the images.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
clients_src=${1:-}

run="thevault-imgtest-$(date +%s)-$$"
image_plain="$run:plain"
image_clients="$run:clients"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/$run.XXXXXX")
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' Cargo.toml)
password="pw-$run-secret"
export VAULT_PASSWORD="$password"
failures=0
images=()

cleanup() {
  docker ps -aq --filter "label=$run" | xargs -r docker rm -f -v > /dev/null
  if [ "${KEEP_IMAGES:-}" != 1 ] && [ ${#images[@]} -gt 0 ]; then
    docker rmi -f "${images[@]}" > /dev/null 2>&1 || true
  fi
  # The containers write as uid 1000; when that is not this user, remove the files as root.
  if ! rm -rf "$tmp" 2> /dev/null && [ ${#images[@]} -gt 0 ]; then
    docker run --rm --user 0 --entrypoint rm -v "$tmp:/t" "${images[0]}" -rf /t/data /t/config > /dev/null 2>&1 || true
    rm -rf "$tmp"
  fi
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

# A fresh pair of /data and /config folders under $tmp, named $1.
dirs() {
  mkdir -p "$tmp/$1/data" "$tmp/$1/config"
  # The image runs as uid 1000.
  if [ "$(id -u)" != 1000 ]; then chmod 0777 "$tmp/$1/data" "$tmp/$1/config"; fi
}

# Start a detached container named $run-$1 from image $2 on /data and /config of $3, publishing
# 8080 to a port Docker picks; prints the port.
start() {
  local name="$run-$1" image=$2 set=$3
  shift 3
  docker run -d --name "$name" --label "$run" -e VAULT_PASSWORD \
    -p 127.0.0.1::8080 \
    -v "$tmp/$set/data:/data" -v "$tmp/$set/config:/config" "$@" "$image" > /dev/null
  if ! docker port "$name" 8080/tcp 2> /dev/null | head -n 1 | sed 's/.*://' | grep .; then
    echo "FAIL container $name did not stay up; its log:" >&2
    docker logs "$name" >&2
    return 1
  fi
}

# Wait until GET /api/health on port $1 answers, up to 60 seconds.
wait_health() {
  local i
  for i in $(seq 1 120); do
    if curl -fsS "http://127.0.0.1:$1/api/health" > /dev/null 2>&1; then return 0; fi
    sleep 0.5
  done
  return 1
}

# Wait until Docker's healthcheck of container $1 reports healthy, up to 60 seconds.
wait_healthy() {
  local i
  for i in $(seq 1 60); do
    [ "$(docker inspect -f '{{.State.Health.Status}}' "$1")" = healthy ] && return 0
    sleep 1
  done
  return 1
}

stop() {
  docker stop -t 20 "$run-$1" > /dev/null
}

build() {
  local tag=$1
  shift
  echo "building $tag"
  docker build -f docker/Dockerfile --platform linux/amd64 -t "$tag" "$@" . > "$tmp/build-${tag##*:}.log" 2>&1 ||
    {
      tail -n 40 "$tmp/build-${tag##*:}.log"
      return 1
    }
  images+=("$tag")
}

# The image's size unpacked on disk, and its compressed content as a registry would store it.
size_mb() {
  printf '%s unpacked, %s compressed' "$(docker image ls --format '{{.Size}}' "$1")" \
    "$(docker image inspect -f '{{.Size}}' "$1" | awk '{ printf "%.1fMB", $1 / 1000000 }')"
}

# Login on port $1 with password $2; prints the HTTP status.
login() {
  curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' \
    --data "$(jq -cn --arg p "$2" '{password: $p}')" "http://127.0.0.1:$1/api/login"
}

# The uid that vault-server runs as in container $1.
server_uid() {
  docker exec "$1" sh -c 'for p in /proc/[0-9]*; do
    if [ "$(cat "$p/comm" 2> /dev/null)" = vault-server ]; then
      sed -n "s/^Uid:[[:space:]]*\([0-9]*\).*/\1/p" "$p/status"
    fi
  done'
}

backups() {
  find "$tmp/$1/data/backups" -name '*.sqlar' 2> /dev/null | sort
}

# The entries of the sqlar archive $1.
sqlar_names() {
  python3 -c 'import sqlite3, sys; [print(r[0]) for r in sqlite3.connect(sys.argv[1]).execute("select name from sqlar")]' "$1"
}

echo "image test $run, workspace version $version, temporary folder $tmp"

# 1. The image builds for linux/amd64.
build "$image_plain"
check "image is linux/amd64" test "$(docker image inspect -f '{{.Os}}/{{.Architecture}}' "$image_plain")" = linux/amd64
echo "SIZE without clients: $(size_mb "$image_plain")"

# 2. Empty /data and no password: one line, non-zero exit.
dirs nopw
set +e
out=$(docker run --rm --label "$run" -v "$tmp/nopw/data:/data" -v "$tmp/nopw/config:/config" "$image_plain" 2>&1)
code=$?
set -e
check "no VAULT_PASSWORD exits non-zero (exit $code)" test "$code" -ne 0
check "no VAULT_PASSWORD prints one line" test "$(printf '%s\n' "$out" | wc -l)" -eq 1
check "the line says to set VAULT_PASSWORD in .env: $out" grep -q 'set VAULT_PASSWORD in .env' <<< "$out"
check "no vault was created" test ! -e "$tmp/nopw/data/.vault"

# 3. First run with VAULT_PASSWORD.
dirs main
port=$(start first "$image_plain" main)
check "/api/health answers on 127.0.0.1:$port" wait_health "$port"
check "login with VAULT_PASSWORD answers 200" test "$(login "$port" "$password")" = 200
check "login with a wrong password answers 401" test "$(login "$port" wrong)" = 401
got=$(curl -fsS "http://127.0.0.1:$port/api/version" | jq -r .server_version)
check "/api/version reports $version (got $got)" test "$got" = "$version"
check "the web client is served at /" sh -c "curl -fsS 'http://127.0.0.1:$port/' | grep -qi '<html'"
got=$(curl -fsS "http://127.0.0.1:$port/api/clients" | jq -c .clients)
check "an image without clients lists none (got $got)" test "$got" = "[]"
check "the healthcheck turns healthy" wait_healthy "$run-first"
uid=$(server_uid "$run-first")
check "vault-server runs as uid 1000 (got $uid)" test "$uid" = 1000
check "the secret key is at /config/vault.key" test -s "$tmp/main/config/vault.key"
check "the vault records version $version" test "$(cat "$tmp/main/data/.vault/server-version")" = "$version"
check "a first run writes no backup" test -z "$(backups main)"
check "the logs never show the password" sh -c "! docker logs '$run-first' 2>&1 | grep -qF '$password'"
stop first
check "the server stops on docker stop with exit 0 (got $(docker inspect -f '{{.State.ExitCode}}' "$run-first"))" \
  test "$(docker inspect -f '{{.State.ExitCode}}' "$run-first")" = 0

# 4. The vault last ran an older version: one backup, then none.
echo 0.0.1 > "$tmp/main/data/.vault/server-version"
port=$(start upgrade "$image_plain" main)
check "the upgraded server answers" wait_health "$port"
mapfile -t found < <(backups main)
check "an upgrade start writes one backup (${found[*]:-none})" test ${#found[@]} -eq 1
if [ ${#found[@]} -eq 1 ]; then
  check "the backup is named 0.0.1-to-$version-<time>.sqlar" \
    grep -qE "/backups/0\.0\.1-to-${version//./\\.}-[0-9]{8}T[0-9]{6}Z\.sqlar$" <<< "${found[0]}"
  names=$(sqlar_names "${found[0]}")
  check "the backup holds state.db" grep -qx '.vault/state.db' <<< "$names"
  check "the backup leaves the secret key out" sh -c "! grep -q '^key/' <<< '$names'"
  check "the backup path is logged" sh -c "docker logs '$run-upgrade' 2>&1 | grep -qF 'backups/$(basename "${found[0]}")'"
fi
check "login still works after the upgrade" test "$(login "$port" "$password")" = 200
stop upgrade
port=$(start again "$image_plain" main)
check "the second start answers" wait_health "$port"
check "a second start writes no backup" test "$(backups main | wc -l)" -eq 1
check "a second start logs no backup" sh -c "! docker logs '$run-again' 2>&1 | grep -q 'backed up'"
stop again

# Only the newest five upgrade backups stay.
for i in 1 2 3 4 5; do
  : > "$tmp/main/data/backups/0.0.0-to-0.0.$i-20000101T00000${i}Z.sqlar"
  touch -d "2000-01-01 00:00:0$i" "$tmp/main/data/backups/0.0.0-to-0.0.$i-20000101T00000${i}Z.sqlar"
done
echo 0.0.2 > "$tmp/main/data/.vault/server-version"
port=$(start prune "$image_plain" main)
check "the pruning start answers" wait_health "$port"
check "five backups stay" test "$(backups main | wc -l)" -eq 5
check "the two oldest went" sh -c "! ls '$tmp/main/data/backups' | grep -qE '0\.0\.[12]-2000'"
stop prune

# A vault from a newer version starts, without a backup.
echo 99.0.0 > "$tmp/main/data/.vault/server-version"
port=$(start newer "$image_plain" main)
check "a vault from a newer version starts" wait_health "$port"
check "a vault from a newer version gets no backup" test "$(backups main | wc -l)" -eq 5
check "a vault from a newer version records $version (got $(cat "$tmp/main/data/.vault/server-version"))" \
  test "$(cat "$tmp/main/data/.vault/server-version")" = "$version"
stop newer

# Versions compare as semver, a pre-release before its release (ticket 149): the image's version
# is set by mounting a VERSION file over the one baked in.
dirs semver
# Start container $run-$1 on the semver set with the image reporting version $2, wait for it, stop it.
semver_start() {
  printf '%s\n' "$2" > "$tmp/semver/VERSION-$2"
  local port
  port=$(start "$1" "$image_plain" semver -v "$tmp/semver/VERSION-$2:/opt/thevault/VERSION:ro")
  check "the $2 start answers" wait_health "$port"
  stop "$1"
  check "the $2 start records $2 (got $(cat "$tmp/semver/data/.vault/server-version"))" \
    test "$(cat "$tmp/semver/data/.vault/server-version")" = "$2"
}
semver_backups() {
  backups semver | wc -l
}
semver_start s-alpha 0.1.13-alpha.4
check "a first run on 0.1.13-alpha.4 writes no backup" test "$(semver_backups)" -eq 0
semver_start s-stable 0.1.13
check "0.1.13-alpha.4 to 0.1.13 backs up" test "$(semver_backups)" -eq 1
check "the backup is named 0.1.13-alpha.4-to-0.1.13-<time>.sqlar" \
  sh -c "ls '$tmp/semver/data/backups' | grep -qE '^0\.1\.13-alpha\.4-to-0\.1\.13-[0-9]{8}T[0-9]{6}Z\.sqlar$'"
semver_start s-next 0.1.14-alpha.1
check "0.1.13 to 0.1.14-alpha.1 backs up" test "$(semver_backups)" -eq 2
semver_start s-down 0.1.13
check "0.1.14-alpha.1 to 0.1.13 is a downgrade and writes no backup" test "$(semver_backups)" -eq 2
check "the downgrade is logged" sh -c "docker logs '$run-s-down' 2>&1 | grep -q 'newer than this image'"
semver_start s-same 0.1.13
check "the same version again writes no backup" test "$(semver_backups)" -eq 2

# 5. A vault made by `vault init` outside the container, its key elsewhere, mounted in.
dirs outside
mkdir -p "$tmp/outside/hostkeys"
[ "$(id -u)" = 1000 ] || chmod 0777 "$tmp/outside/hostkeys"
printf '%s\n' "$password" | docker run --rm -i --label "$run" --entrypoint vault \
  -e VAULT_SECRET_KEY_FILE=/hostkeys/outside.key \
  -v "$tmp/outside/data:/data" -v "$tmp/outside/hostkeys:/hostkeys" \
  "$image_plain" init /data --password-stdin > /dev/null
port=$(VAULT_PASSWORD='' start outside "$image_plain" outside)
check "a vault made outside starts without VAULT_PASSWORD" wait_health "$port"
check "login on the outside vault works" test "$(login "$port" "$password")" = 200
check "the outside vault was not re-created" sh -c "! docker logs '$run-outside' 2>&1 | grep -q 'created a vault'"
check "an unrecorded version is backed up first" sh -c "ls '$tmp/outside/data/backups' | grep -qE '^unknown-to-'"
check "a missing key is logged, not fatal" sh -c "docker logs '$run-outside' 2>&1 | grep -q 'no secret key at /config/vault.key'"
stop outside

# 6. Clients baked in.
if [ -n "$clients_src" ]; then
  docker/stage-clients.sh "$clients_src" "$tmp/clients" "$version" > /dev/null
  build "$image_clients" --build-context "clients=$tmp/clients"
  echo "SIZE with clients: $(size_mb "$image_clients")"
  dirs withclients
  port=$(start clients "$image_clients" withclients)
  check "the clients image answers" wait_health "$port"
  listed=$(curl -fsS "http://127.0.0.1:$port/api/clients")
  check "/api/clients lists an APK" test "$(jq '[.clients[] | select(.kind == "apk")] | length' <<< "$listed")" -ge 1
  check "/api/clients lists an AppImage" test "$(jq '[.clients[] | select(.kind == "appimage")] | length' <<< "$listed")" -ge 1
  for kind in apk appimage; do
    file=$(jq -r --arg k "$kind" '[.clients[] | select(.kind == $k)][0].filename' <<< "$listed")
    want=$(jq -r --arg f "$file" '.clients[] | select(.filename == $f) | .sha256' "$clients_src/manifest.json")
    got=$(curl -fsS "http://127.0.0.1:$port/api/clients/$file" | sha256sum | cut -d' ' -f1)
    check "downloading $file gives sha256 $want (got $got)" test "$got" = "$want"
  done
  stop clients
  # BuildKit once reused a with-clients layer for a build without them; build plain again, now
  # that a with-clients build is cached, and check it carries none.
  build "$run:plain-again"
  got=$(docker run --rm --label "$run" --entrypoint ls "$run:plain-again" -A /opt/thevault/clients)
  check "a plain build after a clients build carries no clients (got '$got')" test -z "$got"
else
  echo "SKIP clients: no clients-dir given"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed"
