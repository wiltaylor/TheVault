#!/usr/bin/env bash
# End-to-end checks of deploy/compose.yaml, run on a host with Docker:
#
#   docker/test-compose.sh
#
# Builds the image from this checkout (or uses IMAGE=<ref> when given) and tags it as
# ghcr.io/wiltaylor/thevault:<run>, so the compose file runs it unchanged with THEVAULT_VERSION
# set in a test .env. It copies compose.yaml, Caddyfile and .env.example into a temporary folder,
# as a user downloading them from a release would, and checks:
#
# - .env.example documents every variable compose.yaml reads, VAULT_PASSWORD first, and
#   docs/install.md and .github/workflows/image.yml agree on the release file names;
# - `docker compose config` renders the default, https and cloudflare profiles;
# - the default profile comes up healthy on a free port, the login works, and the documented
#   password change, backup and restore work;
# - the https profile, with docker/compose-test/https.yaml switching Caddy to its internal CA and
#   a free port, serves the app over https for a test domain;
# - the cloudflare profile starts cloudflared, which logs that a dummy token is not valid.
#
# Every port is one Docker picks on 127.0.0.1, every container, network and volume belongs to the
# compose project thevault-composetest-<time>-<pid>, and all of them, the image tag and the
# temporary folder are removed when it ends.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

run="thevault-composetest-$(date +%s)-$$"
tag="ghcr.io/wiltaylor/thevault:$run"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/$run.XXXXXX")
password="pw-$run-secret"
password2="pw2-$run-secret"
domain="vault.test"
failures=0
tagged=

dc() {
  docker compose --project-directory "$tmp" -p "$run" -f "$tmp/compose.yaml" "$@"
}

cleanup() {
  dc -f "$tmp/https.yaml" --profile https --profile cloudflare \
    down -v --remove-orphans --timeout 20 > /dev/null 2>&1 || true
  if [ -n "$tagged" ]; then docker rmi "$tag" > /dev/null 2>&1 || true; fi
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

# Login at base URL $1 with password $2, extra curl arguments after; prints the HTTP status.
login() {
  local base=$1 pw=$2
  shift 2
  curl -s -o /dev/null -w '%{http_code}' "$@" -H 'content-type: application/json' \
    --data "$(jq -cn --arg p "$pw" '{password: $p}')" "$base/api/login"
}

# The host port that compose published for service $1's container port $2.
published() {
  local service=$1 target=$2
  shift 2
  dc "$@" port "$service" "$target" | head -n 1 | sed 's/.*://'
}

echo "compose test $run, temporary folder $tmp"

# The image, under the name compose.yaml asks for.
if [ -n "${IMAGE:-}" ]; then
  docker tag "$IMAGE" "$tag"
else
  echo "building the image"
  docker build -f docker/Dockerfile --platform linux/amd64 -t "$tag" . > "$tmp/build.log" 2>&1 ||
    {
      tail -n 40 "$tmp/build.log"
      exit 1
    }
fi
tagged=1

# The release files, and a filled-in .env.
cp deploy/compose.yaml deploy/Caddyfile "$tmp/"
cp deploy/.env.example "$tmp/.env.example"
cp docker/compose-test/https.yaml docker/compose-test/Caddyfile.test "$tmp/"
sed -e "s|^VAULT_PASSWORD=.*|VAULT_PASSWORD=$password|" \
  -e "s|^THEVAULT_VERSION=.*|THEVAULT_VERSION=$run|" \
  -e "s|^THEVAULT_PORT=.*|THEVAULT_PORT=127.0.0.1:|" \
  -e "s|^THEVAULT_DOMAIN=.*|THEVAULT_DOMAIN=$domain|" \
  -e "s|^CLOUDFLARE_TUNNEL_TOKEN=.*|CLOUDFLARE_TUNNEL_TOKEN=dummy-not-a-token|" \
  "$tmp/.env.example" > "$tmp/.env"

# Caddy and cloudflared come from Docker Hub; pull them first so a slow pull is not counted
# against the health waits below.
echo "pulling caddy and cloudflared"
dc --profile https --profile cloudflare pull -q caddy cloudflared

# 1. .env.example documents every variable compose.yaml reads.
used=$(grep -o '\${[A-Z_]*' deploy/compose.yaml | tr -d '${' | sort -u)
for var in $used; do
  check ".env.example sets $var" grep -q "^$var=" deploy/.env.example
  check ".env.example comments $var on the line above" \
    sh -c "grep -B1 '^$var=' deploy/.env.example | head -n 1 | grep -q '^# '"
done
first=$(grep -m1 -E '^[A-Z_]+=' deploy/.env.example | cut -d= -f1)
check "VAULT_PASSWORD is the first variable (got $first)" test "$first" = VAULT_PASSWORD
check ".env.example says how to change the password" grep -q 'docker compose exec thevault vault passwd' deploy/.env.example

# docs/install.md covers each step, and the README links it.
check "README.md links docs/install.md" grep -q '(docs/install.md)' README.md
for want in 'is public, so no registry login is needed' 'releases/latest/download' 'docker compose up -d' \
  'Pair a device' '--profile https' '--profile cloudflare' 'URL**: `thevault:8080`' \
  'integrations/google/callback' 'vault backup' '/data/backups/<old>-to-<new>-<time>.sqlar' \
  '/config/vault.key' 'docker compose pull && docker compose up -d' 'restore-backup' \
  'docker compose down -v' 'docker compose exec thevault vault passwd'; do
  check "docs/install.md mentions $want" grep -qF -- "$want" docs/install.md
done

# The release attaches the files docs/install.md downloads, under names GitHub keeps: it renames a
# release file whose name starts with a dot, so .env.example goes up as env.example.
check "docs/install.md downloads env.example" grep -qF 'curl -fL -o .env "$base/env.example"' docs/install.md
check "docs/install.md downloads no dotted release file" sh -c "! grep -qF '\$base/.' docs/install.md"
check "image.yml attaches deploy/.env.example as env.example" \
  grep -qF 'cp deploy/.env.example "$RUNNER_TEMP/env.example"' .github/workflows/image.yml
for f in '"$RUNNER_TEMP/env.example"' deploy/Caddyfile deploy/compose.yaml; do
  check "image.yml uploads $f" sh -c "grep 'gh release upload' .github/workflows/image.yml | grep -qF -- '$f'"
done

# 2. docker compose config renders each profile.
check "config renders the default profile" dc config -q
check "config renders --profile https" dc --profile https config -q
check "config renders --profile cloudflare" dc --profile cloudflare config -q
got=$(dc config --services | sort | tr '\n' ' ')
check "the default profile runs only thevault (got $got)" test "$got" = "thevault "
got=$(dc --profile https config --services | sort | tr '\n' ' ')
check "--profile https adds caddy (got $got)" test "$got" = "caddy thevault "
got=$(dc --profile cloudflare config --services | sort | tr '\n' ' ')
check "--profile cloudflare adds cloudflared (got $got)" test "$got" = "cloudflared thevault "
got=$(dc --profile cloudflare config --format json | jq -r '.services.cloudflared.environment.TUNNEL_TOKEN')
check "cloudflared gets CLOUDFLARE_TUNNEL_TOKEN as TUNNEL_TOKEN" test "$got" = dummy-not-a-token
got=$(dc --profile cloudflare config --format json | jq -c '.services.cloudflared.ports // []')
check "cloudflared publishes no host ports (got $got)" test "$got" = "[]"
got=$(dc --profile cloudflare config --format json | jq -r '.services.cloudflared.depends_on.thevault.condition')
check "cloudflared waits for thevault to be healthy (got $got)" test "$got" = service_healthy
got=$(dc -f "$tmp/https.yaml" --profile https config --format json | jq -c '[.services.caddy.ports[].published]')
check "the test override replaces Caddy's host ports (got $got)" sh -c "! grep -qE '\"(80|443)\"' <<< '$got'"
got=$(COMPOSE_PROFILES=https dc config --services | sort | tr '\n' ' ')
check "COMPOSE_PROFILES=https adds caddy without --profile (got $got)" test "$got" = "caddy thevault "
# A plain .env with host folders renders those folders as bind mounts.
got=$(THEVAULT_DATA=./data THEVAULT_CONFIG=./config dc config --format json | jq -r '[.services.thevault.volumes[].type] | join(",")')
check "THEVAULT_DATA and THEVAULT_CONFIG switch to host folders (got $got)" test "$got" = bind,bind

# 3. The default profile.
echo "starting the default profile"
if ! dc up -d --wait --wait-timeout 120 > "$tmp/up-default.log" 2>&1; then
  cat "$tmp/up-default.log"
  dc logs
  fail "the default profile comes up healthy"
else
  pass "the default profile comes up healthy"
fi
port=$(published thevault 8080)
base="http://127.0.0.1:$port"
check "/api/health answers on a free port ($port)" retry 60 curl -fsS "$base/api/health"
check "login with VAULT_PASSWORD answers 200" test "$(login "$base" "$password")" = 200
check "login with a wrong password answers 401" test "$(login "$base" wrong)" = 401
check "the vault is in the named volume ${run}_data" sh -c "docker volume inspect '${run}_data' > /dev/null"
check "the key is in the named volume ${run}_config" dc exec -T thevault test -s /config/vault.key
check "the app port is bound to 127.0.0.1 only" \
  sh -c "docker port '$(dc ps -q thevault)' 8080/tcp | grep -qvE '^0\.0\.0\.0|^\[::\]'"

# The documented password change.
printf '%s\n' "$password2" | dc exec -T thevault vault passwd --password-stdin > "$tmp/passwd.json"
check "vault passwd changes the password" test "$(login "$base" "$password2")" = 200
check "the old password stops working" test "$(login "$base" "$password")" = 401

# The documented backup, copied out of the volume.
dc exec -T thevault vault backup /data/backups/manual.sqlar > "$tmp/backup.json" 2>&1
check "vault backup writes /data/backups/manual.sqlar" dc exec -T thevault test -s /data/backups/manual.sqlar
dc cp thevault:/data/backups/manual.sqlar "$tmp/manual.sqlar" > /dev/null 2>&1
check "docker compose cp copies the backup out" test -s "$tmp/manual.sqlar"

# The documented restore: a new data volume, the backup unpacked into it, the same key.
dc down --timeout 20 > /dev/null 2>&1
docker volume rm "${run}_data" > /dev/null
dc run --rm --no-deps -T -v "$tmp/manual.sqlar:/restore.sqlar:ro" --entrypoint vault thevault \
  restore-backup /restore.sqlar /data > "$tmp/restore.json" 2> "$tmp/restore.err" || cat "$tmp/restore.json" "$tmp/restore.err"
check "vault restore-backup fills the new volume ($(jq -c . "$tmp/restore.json" 2> /dev/null))" sh -c "jq -e '.error == null' '$tmp/restore.json' > /dev/null"
dc up -d --wait --wait-timeout 120 > /dev/null 2>&1 || fail "the restored vault comes up healthy"
port=$(published thevault 8080)
base="http://127.0.0.1:$port"
check "the restored vault answers" retry 60 curl -fsS "$base/api/health"
check "login after the restore takes the changed password" test "$(login "$base" "$password2")" = 200
check "the restore did not re-create the vault" sh -c "! docker compose --project-directory '$tmp' -p '$run' -f '$tmp/compose.yaml' logs thevault 2>&1 | grep -q 'created a vault'"
dc down --timeout 20 > /dev/null 2>&1

# 4. The https profile, Caddy on its internal CA and a free port.
https=(-f "$tmp/https.yaml" --profile https)
echo "starting the https profile"
if ! dc "${https[@]}" up -d --wait --wait-timeout 120 > "$tmp/up-https.log" 2>&1; then
  cat "$tmp/up-https.log"
  dc "${https[@]}" logs
  fail "the https profile comes up"
else
  pass "the https profile comes up"
fi
sport=$(published caddy 443 "${https[@]}")
retry 30 dc "${https[@]}" cp caddy:/data/caddy/pki/authorities/local/root.crt "$tmp/caddy-root.crt" || true
check "Caddy made its local CA" test -s "$tmp/caddy-root.crt"
tls=(--cacert "$tmp/caddy-root.crt" --resolve "$domain:$sport:127.0.0.1")
sbase="https://$domain:$sport"
check "https://$domain/api/health answers through Caddy, the certificate verified" retry 60 curl -fsS "${tls[@]}" "$sbase/api/health"
check "login over https answers 200" test "$(login "$sbase" "$password2" "${tls[@]}")" = 200
check "the web client is served over https" sh -c "curl -fsS --cacert '$tmp/caddy-root.crt' --resolve '$domain:$sport:127.0.0.1' '$sbase/' | grep -qi '<html'"
check "the certificate names $domain" sh -c "curl -sv --cacert '$tmp/caddy-root.crt' --resolve '$domain:$sport:127.0.0.1' '$sbase/api/health' 2>&1 | grep -qi 'subjectAltName.*$domain\|$domain.*matched'"
dc "${https[@]}" down --timeout 20 > /dev/null 2>&1

# 5. The cloudflare profile with a dummy token.
echo "starting the cloudflare profile"
dc --profile cloudflare up -d > "$tmp/up-cloudflare.log" 2>&1 || cat "$tmp/up-cloudflare.log"
cf_logs() { dc --profile cloudflare logs cloudflared 2>&1; }
cf_says_invalid() { cf_logs | grep -qi 'token.*not valid\|invalid.*token'; }
check "cloudflared starts once thevault is healthy" retry 120 sh -c "test -n \"\$(docker compose --project-directory '$tmp' -p '$run' -f '$tmp/compose.yaml' --profile cloudflare ps -a -q cloudflared)\""
check "cloudflared logs that the dummy token is not valid" retry 60 cf_says_invalid
echo "cloudflared said: $(cf_logs | grep -i -m1 'token' || true)"
dc --profile cloudflare down --timeout 20 > /dev/null 2>&1

# 6. Nothing is left behind.
cleanup
trap - EXIT
left=$(docker ps -aq --filter "label=com.docker.compose.project=$run"; docker volume ls -q --filter "label=com.docker.compose.project=$run"; docker network ls -q --filter "label=com.docker.compose.project=$run")
check "no container, volume or network of $run is left" test -z "$left"
check "the image tag is removed" sh -c "! docker image inspect '$tag' > /dev/null 2>&1"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed"
