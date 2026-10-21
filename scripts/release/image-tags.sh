#!/usr/bin/env bash
# Print, one per line, the tags a release's server image gets (ticket 139, docs/releasing.md):
#
#     release      <image>:<version>  <image>:<major>.<minor>  <image>:latest  <image>:edge
#     pre-release  <image>:<version>  <image>:edge
#
# so latest and <major>.<minor> only ever name a release, and edge names the newest build of
# either kind. docker/release-image.sh tags and pushes these.
#
#     scripts/release/image-tags.sh <image> <version>
set -euo pipefail

image=${1:-}
version=${2:-}
if [ -z "$image" ] || ! [[ $version =~ ^([0-9]+\.[0-9]+)\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "usage: $0 <image> <X.Y.Z or X.Y.Z-pre>" >&2
  exit 2
fi
echo "$image:$version"
if [ -z "${BASH_REMATCH[2]}" ]; then
  echo "$image:${BASH_REMATCH[1]}"
  echo "$image:latest"
fi
echo "$image:edge"
