#!/usr/bin/env bash
# Build the browser box image (docker/browser.Dockerfile) of this checkout's version, check it with
# docker/test-browser-image.sh, and push it with --push (browser B9). .github/workflows/browser-
# image.yml runs this beside the server image once a release train has published the release;
# run it the same way locally, without --push, to see what CI would publish.
#
#   docker/release-browser-image.sh [--push]
#
# Environment:
#   IMAGE     the repository to tag, default ghcr.io/wiltaylor/thevault-browser
#   TAG       the git tag being released; when set it must be v<version>
#   REVISION  the commit the image is built from, default `git rev-parse HEAD`
#
# The tags are the server image's (scripts/release/image-tags.sh): <version>, <major>.<minor>,
# latest and edge for a release, <version> and edge for a pre-release. Without --push the tagged
# image stays local.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

push=""
case "${1:-}" in
  "") ;;
  --push) push=yes ;;
  *)
    echo "usage: $0 [--push]" >&2
    exit 2
    ;;
esac

image=${IMAGE:-ghcr.io/wiltaylor/thevault-browser}
revision=${REVISION:-$(git rev-parse HEAD)}
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' Cargo.toml)
if [ -n "${TAG:-}" ] && [ "$TAG" != "v$version" ]; then
  echo "tag $TAG does not match the workspace version $version" >&2
  exit 1
fi
mapfile -t tags < <(scripts/release/image-tags.sh "$image" "$version")

echo "building ${tags[*]}"
tag_args=()
for t in "${tags[@]}"; do tag_args+=(-t "$t"); done
docker build -f docker/browser.Dockerfile --platform linux/amd64 \
  --label "org.opencontainers.image.title=TheVault browser box" \
  --label "org.opencontainers.image.description=A desktop with playwright-cli and Chromium that TheVault's agents drive, and its exec service" \
  --label "org.opencontainers.image.source=https://github.com/wiltaylor/TheVault" \
  --label "org.opencontainers.image.url=https://github.com/wiltaylor/TheVault" \
  --label "org.opencontainers.image.version=$version" \
  --label "org.opencontainers.image.revision=$revision" \
  --label "org.opencontainers.image.licenses=MIT" \
  --label "org.opencontainers.image.created=$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${tag_args[@]}" .

# Check it before anything is pushed.
IMAGE="${tags[0]}" docker/test-browser-image.sh

if [ -n "$push" ]; then
  for t in "${tags[@]}"; do docker push "$t"; done
  echo "pushed ${tags[*]}"
else
  echo "built ${tags[*]} locally; not pushed (pass --push to push)"
fi
