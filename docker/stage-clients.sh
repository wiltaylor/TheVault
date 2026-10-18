#!/usr/bin/env bash
# Stage the clients build context for docker/Dockerfile: copy from <src>, a folder that
# `just publish-clients` wrote, the APK, AppImage and static vault CLI builds of <version> into
# <dest>, with a manifest.json listing only them. <src> is only read.
#
# Usage: docker/stage-clients.sh <src> <dest> <version>
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <src> <dest> <version>" >&2
  exit 2
fi
src=$1 dest=$2 version=$3

if [ ! -f "$src/manifest.json" ]; then
  echo "$src has no manifest.json: publish the clients there first (just publish-clients $src)" >&2
  exit 1
fi
mkdir -p "$dest"
jq --arg v "$version" \
  '{clients: [.clients[] | select(.version == $v and (.kind == "apk" or .kind == "appimage" or .kind == "binary"))]}' \
  "$src/manifest.json" > "$dest/manifest.json"
mapfile -t files < <(jq -r '.clients[].filename' "$dest/manifest.json")
if [ ${#files[@]} -eq 0 ]; then
  echo "$src/manifest.json lists no APK, AppImage or CLI build of version $version" >&2
  exit 1
fi
for f in "${files[@]}"; do
  cp "$src/$f" "$dest/$f"
  echo "staged $f"
done
