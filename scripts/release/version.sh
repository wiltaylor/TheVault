#!/usr/bin/env bash
# Print the version a release train cuts, from the tags of the git repository in the current
# directory (ticket 139, docs/releasing.md).
#
#     scripts/release/version.sh <pre-release|release> [<X.Y.Z>]
#
# The base is the highest stable tag vX.Y.Z; pre-release tags (v0.1.13-alpha.1) never count as
# the base. The target is the base's next patch, or <X.Y.Z> (the `version:` trailer), which must
# be newer than the base. A release is the target itself; a pre-release is <target>-alpha.N,
# where N is one more than the highest alpha tag of the target, from 1 to 98 (the Android
# versionCode has two digits for it: scripts/release/version-code.sh). Exits 1, printing why,
# when the tag of the version already exists.
set -euo pipefail

fail() {
  echo "release version: $*" >&2
  exit 1
}

kind=${1:-}
want=${2:-}
case "$kind" in
  pre-release | release) ;;
  *)
    echo "usage: $0 <pre-release|release> [<X.Y.Z>]" >&2
    exit 2
    ;;
esac

number='(0|[1-9][0-9]*)'
stable="^v$number\\.$number\\.$number\$"
base=$(git tag --list 'v*' | grep -E "$stable" | sort -V | tail -n 1 || true)
base=${base#v}
base=${base:-0.0.0}
IFS=. read -r major minor patch <<< "$base"

if [ -n "$want" ]; then
  want=${want#v}
  [[ $want =~ ^$number\.$number\.$number$ ]] || fail "the version trailer $want is not X.Y.Z"
  if [ "$kind" = release ] && git rev-parse -q --verify "refs/tags/v$want" > /dev/null; then
    fail "the tag v$want already exists"
  fi
  newest=$(printf '%s\n%s\n' "$base" "$want" | sort -V | tail -n 1)
  if [ "$want" = "$base" ] || [ "$newest" != "$want" ]; then
    fail "the version trailer $want is not newer than the last release v$base"
  fi
  target=$want
else
  target="$major.$minor.$((patch + 1))"
fi

if [ "$kind" = release ]; then
  version=$target
else
  last=$(git tag --list "v$target-alpha.*" | sed -n "s/^v${target//./\\.}-alpha\\.\\([1-9][0-9]*\\)\$/\\1/p" | sort -n | tail -n 1)
  n=$((${last:-0} + 1))
  [ "$n" -le 98 ] || fail "$target already has 98 alphas; release it or set a newer version trailer"
  version="$target-alpha.$n"
fi

if git rev-parse -q --verify "refs/tags/v$version" > /dev/null; then
  fail "the tag v$version already exists"
fi
echo "$version"
