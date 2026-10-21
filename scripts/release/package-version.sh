#!/usr/bin/env bash
# Print a release version as one Linux package format writes it, so a pre-release sorts before
# its release in that format's package manager (ticket 139, docs/releasing.md):
#
#     deb  0.1.13-alpha.1 -> 0.1.13~alpha.1   dpkg sorts ~ before anything, even the end
#     rpm  0.1.13-alpha.1 -> 0.1.13~alpha.1   rpm 4.10 and later sort ~ the same way
#     aur  0.1.13-alpha.1 -> 0.1.13alpha1     pacman sorts a letter run before the end
#
# A release, 0.1.13, is the same in every format.
#
#     scripts/release/package-version.sh <deb|rpm|aur> <version>
set -euo pipefail

format=${1:-}
version=${2:-}
if ! [[ $version =~ ^([0-9]+\.[0-9]+\.[0-9]+)(-([0-9A-Za-z.]+))?$ ]]; then
  echo "usage: $0 <deb|rpm|aur> <X.Y.Z or X.Y.Z-pre>; not $version" >&2
  exit 2
fi
core=${BASH_REMATCH[1]} pre=${BASH_REMATCH[3]}
case "$format" in
  deb | rpm) echo "$core${pre:+~$pre}" ;;
  aur) echo "$core${pre//./}" ;;
  *)
    echo "usage: $0 <deb|rpm|aur> <version>" >&2
    exit 2
    ;;
esac
