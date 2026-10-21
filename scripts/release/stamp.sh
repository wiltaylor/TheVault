#!/usr/bin/env bash
# Write a release version into the checkout, as a release train does before it builds (ticket
# 139, docs/releasing.md); main itself keeps the version it has, and nothing is committed.
#
#     scripts/release/stamp.sh <version>        # 0.1.13 or 0.1.13-alpha.1
#
# It sets the version of [workspace.package] in Cargo.toml and of [package] in both app crates,
# the version of every crate those name in the three Cargo.lock files, the version in both
# tauri.conf.json files, and the Android bundle.android.versionCode from
# scripts/release/version-code.sh. THEVAULT_ROOT names another checkout to stamp.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=${THEVAULT_ROOT:-$(cd "$here/../.." && pwd)}
version=${1:-}
code=$("$here/version-code.sh" "$version")
cd "$root"

# [workspace.package] in Cargo.toml, [package] in the app crates.
set_toml() { # <file> <table>
  local file=$1 table=$2 tmp
  tmp=$(mktemp)
  awk -v table="[$table]" -v version="$version" '
    /^\[/ { inside = ($0 == table) }
    inside && !done && /^version *= *"/ { print "version = \"" version "\""; done = 1; next }
    { print }
    END { if (!done) exit 1 }
  ' "$file" > "$tmp" || { rm -f "$tmp"; echo "$file has no version in [$table]" >&2; exit 1; }
  cat "$tmp" > "$file"
  rm -f "$tmp"
}
set_toml Cargo.toml workspace.package
apps=(apps/desktop/src-tauri apps/android/src-tauri)
for app in "${apps[@]}"; do set_toml "$app/Cargo.toml" package; done

# The crates that carry the version: the workspace's (version.workspace = true) and the apps.
names=()
for toml in crates/*/Cargo.toml; do
  if grep -q '^version\.workspace *= *true' "$toml"; then
    names+=("$(sed -n 's/^name *= *"\([^"]*\)".*/\1/p' "$toml" | head -n 1)")
  fi
done
for app in "${apps[@]}"; do
  names+=("$(sed -n 's/^name *= *"\([^"]*\)".*/\1/p' "$app/Cargo.toml" | head -n 1)")
done

# Their entries in each Cargo.lock: a [[package]] of one of those names with no source line.
for lock in Cargo.lock apps/desktop/src-tauri/Cargo.lock apps/android/src-tauri/Cargo.lock; do
  tmp=$(mktemp)
  awk -v version="$version" -v list="${names[*]}" '
    BEGIN { n = split(list, a, " "); for (i = 1; i <= n; i++) ours[a[i]] = 1 }
    function flush() {
      if (count == 0) return
      local_ = 1
      for (i = 1; i <= count; i++) if (block[i] ~ /^source = /) local_ = 0
      for (i = 1; i <= count; i++) {
        if (local_ && (name in ours) && block[i] ~ /^version = "/) print "version = \"" version "\""
        else print block[i]
      }
      count = 0; name = ""
    }
    /^\[\[package\]\]$/ { flush() }
    /^\[\[package\]\]$/ || count > 0 {
      block[++count] = $0
      if ($0 ~ /^name = "/) { name = $0; sub(/^name = "/, "", name); sub(/"$/, "", name) }
      if ($0 == "") flush()
      next
    }
    { print }
    END { flush() }
  ' "$lock" > "$tmp"
  cat "$tmp" > "$lock"
  rm -f "$tmp"
done

# Both tauri.conf.json files, and the Android versionCode.
for conf in apps/desktop/src-tauri/tauri.conf.json apps/android/src-tauri/tauri.conf.json; do
  tmp=$(mktemp)
  if [[ $conf == apps/android/* ]]; then
    jq --arg v "$version" --argjson code "$code" '.version = $v | .bundle.android.versionCode = $code' "$conf" > "$tmp"
  else
    jq --arg v "$version" '.version = $v' "$conf" > "$tmp"
  fi
  cat "$tmp" > "$conf"
  rm -f "$tmp"
done
echo "stamped $version (Android versionCode $code) into $root"
