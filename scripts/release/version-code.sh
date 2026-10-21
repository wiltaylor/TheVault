#!/usr/bin/env bash
# Print the Android versionCode of a release version (ticket 139, docs/releasing.md):
#
#     ((major * 1000 + minor) * 1000 + patch) * 100 + N    for X.Y.Z-alpha.N, N from 1 to 98
#     ((major * 1000 + minor) * 1000 + patch) * 100 + 99   for the release X.Y.Z
#
# so each alpha sorts after the release before it and before its own release, and every code
# exceeds 1012, the code Tauri gave 0.1.12 (major * 1000000 + minor * 1000 + patch), the last
# release before this scheme. Google Play's limit, 2100000000, allows majors up to 20.
#
#     scripts/release/version-code.sh 0.1.13-alpha.1    # 101301
set -euo pipefail

version=${1:-}
number='(0|[1-9][0-9]{0,2})'
if ! [[ $version =~ ^$number\.$number\.$number(-alpha\.([1-9][0-9]?))?$ ]]; then
  echo "usage: $0 <X.Y.Z or X.Y.Z-alpha.N>: minor and patch below 1000, N from 1 to 98; not $version" >&2
  exit 2
fi
major=${BASH_REMATCH[1]} minor=${BASH_REMATCH[2]} patch=${BASH_REMATCH[3]} n=${BASH_REMATCH[5]:-99}
if [ -n "${BASH_REMATCH[4]}" ] && [ "$n" -gt 98 ]; then
  echo "$version: an alpha number runs from 1 to 98" >&2
  exit 2
fi
code=$((((major * 1000 + minor) * 1000 + patch) * 100 + n))
if [ "$code" -gt 2100000000 ]; then
  echo "$version gives versionCode $code, over Android's 2100000000" >&2
  exit 2
fi
echo "$code"
