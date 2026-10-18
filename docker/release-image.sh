#!/usr/bin/env bash
# Build the release image of this checkout's version with the release's APK, AppImage and static
# vault CLI builds baked in,
# check it, and push it with --push. .github/workflows/image.yml runs this on a v* tag once the
# desktop and Android workflows have attached their builds to the release; run it the same way
# locally, without --push, to see exactly what CI would publish (D10, ticket 129).
#
#   docker/release-image.sh <assets-dir> [--push]
#
# <assets-dir> holds the release's files as `gh release download` writes them, only read:
#   TheVault_<version>_aarch64.apk, TheVault_<version>_amd64.AppImage (and its .sig when the
#   release has one), vault-<version>-linux-x86_64 and vault-<version>-linux-aarch64 (`just
#   cli-release`), and the SHA256SUMS files that list them.
#
# Environment:
#   IMAGE     the repository to tag, default ghcr.io/wiltaylor/thevault
#   TAG       the git tag being released; when set it must be v<version>
#   REVISION  the commit the image is built from, default `git rev-parse HEAD`
#
# Tags <version>, <major>.<minor> and latest. Without --push the tagged image stays local.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

assets=${1:-}
push=""
case "${2:-}" in
  "") ;;
  --push) push=yes ;;
  *)
    echo "usage: $0 <assets-dir> [--push]" >&2
    exit 2
    ;;
esac
if [ -z "$assets" ] || [ ! -d "$assets" ]; then
  echo "usage: $0 <assets-dir> [--push]: <assets-dir> must be a folder of the release's files" >&2
  exit 2
fi
assets=$(cd "$assets" && pwd)

image=${IMAGE:-ghcr.io/wiltaylor/thevault}
revision=${REVISION:-$(git rev-parse HEAD)}

# 1. The workspace version, which the apps and the tag must match.
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' Cargo.toml)
for conf in apps/desktop/src-tauri/tauri.conf.json apps/android/src-tauri/tauri.conf.json; do
  app=$(jq -r .version "$conf")
  if [ "$app" != "$version" ]; then
    echo "$conf has version $app, the workspace (Cargo.toml) $version: release them together" >&2
    exit 1
  fi
done
if [ -n "${TAG:-}" ] && [ "$TAG" != "v$version" ]; then
  echo "tag $TAG does not match the workspace version $version" >&2
  exit 1
fi
minor=$(cut -d. -f1-2 <<< "$version")
tags=("$image:$version" "$image:$minor" "$image:latest")

# 2. The release's builds of this version, matching the release's SHA256SUMS.
apk="TheVault_${version}_aarch64.apk"
appimage="TheVault_${version}_amd64.AppImage"
clis=("vault-${version}-linux-x86_64" "vault-${version}-linux-aarch64")
builds=("$apk" "$appimage" "${clis[@]}")
for f in "${builds[@]}"; do
  if [ ! -f "$assets/$f" ]; then
    echo "$assets has no $f: the desktop (with the CLI) and Android workflows attach it to the release" >&2
    exit 1
  fi
done
shopt -s nullglob
sums=("$assets"/SHA256SUMS*)
shopt -u nullglob
if [ ${#sums[@]} -eq 0 ]; then
  echo "$assets has no SHA256SUMS file to check ${builds[*]} against" >&2
  exit 1
fi
for f in "${builds[@]}"; do
  want=$(cat "${sums[@]}" | awk -v f="$f" '$2 == f || $2 == "*" f { print $1; exit }')
  got=$(sha256sum "$assets/$f" | cut -d' ' -f1)
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    echo "$f has sha256 $got, the release's SHA256SUMS ${want:-does not list it}" >&2
    exit 1
  fi
  echo "checked $f against SHA256SUMS"
done

run="thevault-release-$(date +%s)-$$"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/$run.XXXXXX")
plain="$run:plain"
cleanup() {
  docker ps -aq --filter "label=$run" | xargs -r docker rm -f -v > /dev/null
  docker rmi -f "$plain" > /dev/null 2>&1 || true
  # The smoke container writes as uid 1000; when that is not this user, remove its files as root.
  if ! rm -rf "$tmp" 2> /dev/null; then
    docker run --rm --user 0 --entrypoint rm -v "$tmp:/t" "${tags[0]}" -rf /t/data /t/config > /dev/null 2>&1 || true
    rm -rf "$tmp"
  fi
}
trap cleanup EXIT

build() {
  docker build -f docker/Dockerfile --platform linux/amd64 \
    --build-arg "THEVAULT_WEB_COMMIT=${revision:0:7}" "$@" .
}

# 3. The image without clients, whose `vault` writes the clients manifest.
echo "building $plain"
build -t "$plain"

# 4. The clients folder and manifest, as `just publish-clients` writes them: release builds, with
# the AppImage's updater signature when the release has one, and the CLI builds.
mkdir -p "$tmp/clients"
docker run --rm --label "$run" --user "$(id -u):$(id -g)" --entrypoint vault \
  -v "$assets:/assets:ro" -v "$tmp/clients:/clients" "$plain" \
  clients publish --dir /clients --build release "${builds[@]/#//assets/}" > /dev/null
docker/stage-clients.sh "$tmp/clients" "$tmp/stage" "$version"

# 5. The release image, with its OCI labels.
echo "building ${tags[*]}"
tag_args=()
for t in "${tags[@]}"; do tag_args+=(-t "$t"); done
build --build-context "clients=$tmp/stage" \
  --label "org.opencontainers.image.title=TheVault" \
  --label "org.opencontainers.image.description=TheVault note server, its web client and the matching desktop and Android apps" \
  --label "org.opencontainers.image.source=https://github.com/wiltaylor/TheVault" \
  --label "org.opencontainers.image.url=https://github.com/wiltaylor/TheVault" \
  --label "org.opencontainers.image.version=$version" \
  --label "org.opencontainers.image.revision=$revision" \
  --label "org.opencontainers.image.licenses=MIT" \
  --label "org.opencontainers.image.created=$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${tag_args[@]}"

# 6. Check it before anything is pushed: it starts, reports the version, and serves both apps and
# both CLI builds.
mkdir -p "$tmp/data" "$tmp/config"
if [ "$(id -u)" != 1000 ]; then chmod 0777 "$tmp/data" "$tmp/config"; fi
docker run -d --name "$run" --label "$run" -e VAULT_PASSWORD="pw-$run" -p 127.0.0.1::8080 \
  -v "$tmp/data:/data" -v "$tmp/config:/config" "${tags[0]}" > /dev/null
port=$(docker port "$run" 8080/tcp | head -n 1 | sed 's/.*://')
for _ in $(seq 1 120); do
  curl -fsS "http://127.0.0.1:$port/api/health" > /dev/null 2>&1 && break
  sleep 0.5
done
fail() {
  echo "FAIL $*" >&2
  docker logs "$run" >&2 || true
  exit 1
}
got=$(curl -fsS "http://127.0.0.1:$port/api/version" | jq -r .server_version) || fail "the image does not answer /api/version"
[ "$got" = "$version" ] || fail "/api/version reports $got, not $version"
listed=$(curl -fsS "http://127.0.0.1:$port/api/clients")
for pair in "apk:$apk" "appimage:$appimage"; do
  kind=${pair%%:*} f=${pair#*:}
  want=$(sha256sum "$assets/$f" | cut -d' ' -f1)
  # `vault clients publish` names each build by its platform's architecture, so find it by kind.
  served=$(jq -r --arg k "$kind" --arg v "$version" \
    '[.clients[] | select(.kind == $k and .version == $v and .build == "release")][0].filename // empty' <<< "$listed")
  [ -n "$served" ] || fail "/api/clients lists no $kind release of $version: $(jq -c '[.clients[].filename]' <<< "$listed")"
  got=$(curl -fsS "http://127.0.0.1:$port/api/clients/$served" | sha256sum | cut -d' ' -f1)
  [ "$got" = "$want" ] || fail "downloading $served gives sha256 $got, not $f's $want"
  echo "the image serves $f as /api/clients/$served"
done
for f in "${clis[@]}"; do
  arch=${f##*-linux-}
  got=$(curl -fsS "http://127.0.0.1:$port/api/clients/cli/linux-$arch" | sha256sum | cut -d' ' -f1)
  [ "$got" = "$(sha256sum "$assets/$f" | cut -d' ' -f1)" ] || fail "/api/clients/cli/linux-$arch does not serve $f"
  echo "the image serves $f as /api/clients/cli/linux-$arch"
done

docker rm -f -v "$run" > /dev/null
for key in source version revision licenses; do
  echo "label org.opencontainers.image.$key=$(docker image inspect -f "{{index .Config.Labels \"org.opencontainers.image.$key\"}}" "${tags[0]}")"
done

# 7. Push, or say what would be pushed.
if [ -n "$push" ]; then
  for t in "${tags[@]}"; do docker push "$t"; done
  echo "pushed ${tags[*]}"
else
  echo "built ${tags[*]} locally; not pushed (pass --push to push)"
fi
