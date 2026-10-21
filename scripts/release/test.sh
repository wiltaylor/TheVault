#!/usr/bin/env bash
# Tests of the release train's scripts (ticket 139): the version calculation, the Android
# versionCode, the Linux package versions and how each package manager sorts them, the image
# tags, and stamping a checkout. Every git repository and copy lives in a temporary folder.
# Run it through ./dev, which has dpkg, rpm and vercmp: ./dev just ci::release-test
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0
pass() { echo "ok   $*"; }
fail() {
  echo "FAIL $*" >&2
  failed=1
}
expect() { # <what> <want> <got>
  if [ "$2" = "$3" ]; then pass "$1: $3"; else fail "$1: want $2, got $3"; fi
}

# A repository whose history carries the given tags, each on its own commit.
repo() {
  rm -rf "$tmp/repo" && mkdir -p "$tmp/repo"
  git -C "$tmp/repo" init -q
  local tag
  for tag in "$@"; do
    git -C "$tmp/repo" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "$tag"
    git -C "$tmp/repo" tag "$tag"
  done
  git -C "$tmp/repo" -c user.name=t -c user.email=t@t commit -q --allow-empty -m head
}
version() { (cd "$tmp/repo" && "$here/version.sh" "$@" 2> "$tmp/err"); }

echo "# version.sh"
repo v0.1.11 v0.1.12
expect "first pre-release after v0.1.12" 0.1.13-alpha.1 "$(version pre-release)"
expect "first release after v0.1.12" 0.1.13 "$(version release)"
repo v0.1.12 v0.1.13-alpha.1
expect "next pre-release" 0.1.13-alpha.2 "$(version pre-release)"
repo v0.1.12 v0.1.13-alpha.1 v0.1.13-alpha.2
expect "a release after the alphas" 0.1.13 "$(version release)"
repo v0.1.12 v0.1.13-alpha.9 v0.1.13-alpha.10 v0.1.13-alpha.2
expect "alphas count by number, not text" 0.1.13-alpha.11 "$(version pre-release)"
repo v0.1.12 v0.1.13-alpha.1 v0.1.13
expect "a pre-release after a release starts the next patch" 0.1.14-alpha.1 "$(version pre-release)"
expect "the base is the newest release, not the newest tag" 0.1.14 "$(version release)"
repo v0.1.9 v0.1.12 v0.1.10
expect "the base sorts by version" 0.1.13 "$(version release)"
repo v0.1.12 v0.1.13-alpha.1
expect "a version trailer sets a release" 0.2.0 "$(version release 0.2.0)"
expect "a version trailer sets a pre-release" 0.2.0-alpha.1 "$(version pre-release 0.2.0)"
expect "a version trailer may be the next patch" 0.1.13-alpha.2 "$(version pre-release 0.1.13)"
expect "a version trailer may start with v" 1.0.0 "$(version release v1.0.0)"
repo v0.1.12 v0.2.0
if out=$(version release 0.2.0); then fail "an existing tag: printed $out"; else pass "an existing tag fails: $(cat "$tmp/err")"; fi
grep -q 'v0.2.0 already exists' "$tmp/err" || fail "the existing tag is not named: $(cat "$tmp/err")"
if out=$(version release 0.1.12); then fail "an older trailer: printed $out"; else pass "a trailer not newer than the base fails: $(cat "$tmp/err")"; fi
if out=$(version release 0.2); then fail "a bad trailer: printed $out"; else pass "a trailer that is not X.Y.Z fails: $(cat "$tmp/err")"; fi
repo v0.1.12 v0.1.13-alpha.2 v0.1.13-alpha.1
expect "the highest alpha is found whatever the tag order" 0.1.13-alpha.3 "$(version pre-release)"
repo
expect "with no release tag at all" 0.0.1 "$(version release)"
if version prerelease > /dev/null 2>&1; then fail "a bad kind passed"; else pass "a kind that is neither pre-release nor release fails"; fi

echo "# version-code.sh"
shipped=1012 # aapt2 dump badging TheVault_0.1.12_aarch64.apk: versionCode='1012'
codes=()
for v in 0.1.13-alpha.1 0.1.13-alpha.2 0.1.13-alpha.98 0.1.13 0.1.14-alpha.1 0.2.0-alpha.1 0.2.0 1.0.0; do
  codes+=("$("$here/version-code.sh" "$v")")
done
expect "0.1.13-alpha.1" 101301 "${codes[0]}"
expect "0.1.13" 101399 "${codes[3]}"
expect "0.1.12 under the scheme" 101299 "$("$here/version-code.sh" 0.1.12)"
prev=$shipped
for i in "${!codes[@]}"; do
  if [ "${codes[$i]}" -le "$prev" ]; then fail "code ${codes[$i]} does not exceed $prev"; fi
  prev=${codes[$i]}
done
pass "every code exceeds the shipped $shipped and the one before it: ${codes[*]}"
for bad in 0.1.13-alpha.99 0.1.13-beta.1 0.1.13-alpha.0 1000.0.0-alpha.1 0.1000.0; do
  if "$here/version-code.sh" "$bad" > /dev/null 2>&1; then fail "$bad gave a code"; else pass "$bad has no code"; fi
done
committed=$(jq -r .version "$root/apps/android/src-tauri/tauri.conf.json")
expect "the committed Android versionCode is $committed's" "$("$here/version-code.sh" "$committed")" \
  "$(jq -r .bundle.android.versionCode "$root/apps/android/src-tauri/tauri.conf.json")"

echo "# package-version.sh"
expect deb 0.1.13~alpha.1 "$("$here/package-version.sh" deb 0.1.13-alpha.1)"
expect rpm 0.1.13~alpha.1 "$("$here/package-version.sh" rpm 0.1.13-alpha.1)"
expect aur 0.1.13alpha1 "$("$here/package-version.sh" aur 0.1.13-alpha.1)"
expect "a release in aur" 0.1.13 "$("$here/package-version.sh" aur 0.1.13)"
# Each manager must order: 0.1.12 < 0.1.13-alpha.1 < 0.1.13-alpha.2 < 0.1.13-alpha.10 < 0.1.13.
order=(0.1.12 0.1.13-alpha.1 0.1.13-alpha.2 0.1.13-alpha.10 0.1.13)
for format in deb rpm aur; do
  mapped=()
  for v in "${order[@]}"; do mapped+=("$("$here/package-version.sh" "$format" "$v")"); done
  for ((i = 1; i < ${#mapped[@]}; i++)); do
    a=${mapped[$((i - 1))]} b=${mapped[$i]}
    case "$format" in
      deb) dpkg --compare-versions "$a" lt "$b" && r=lt || r=ge ;;
      rpm) [ "$(rpm --eval "%{lua: print(rpm.vercmp('$a', '$b'))}")" = -1 ] && r=lt || r=ge ;;
      aur) [ "$(vercmp "$a" "$b")" = -1 ] && r=lt || r=ge ;;
    esac
    expect "$format $a before $b" lt "$r"
  done
done

echo "# image-tags.sh"
expect "a release's image tags" "i:0.1.13 i:0.1 i:latest i:edge" "$("$here/image-tags.sh" i 0.1.13 | xargs)"
expect "a pre-release's image tags" "i:0.1.13-alpha.1 i:edge" "$("$here/image-tags.sh" i 0.1.13-alpha.1 | xargs)"

echo "# stamp.sh"
copy="$tmp/checkout"
mkdir -p "$copy/apps/desktop/src-tauri" "$copy/apps/android/src-tauri" "$copy/crates"
cp "$root/Cargo.toml" "$root/Cargo.lock" "$copy/"
cp -r "$root/crates/." "$copy/crates/" 2> /dev/null || true
find "$copy/crates" -mindepth 2 ! -name Cargo.toml -delete 2> /dev/null || true
for app in desktop android; do
  cp "$root/apps/$app/src-tauri/"{Cargo.toml,Cargo.lock,tauri.conf.json} "$copy/apps/$app/src-tauri/"
done
THEVAULT_ROOT="$copy" "$here/stamp.sh" 0.1.13-alpha.1 > /dev/null
expect "workspace version" 0.1.13-alpha.1 "$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' "$copy/Cargo.toml")"
for app in desktop android; do
  expect "$app crate version" 0.1.13-alpha.1 "$(sed -n 's/^version = "\(.*\)"/\1/p' "$copy/apps/$app/src-tauri/Cargo.toml" | head -n 1)"
  expect "$app tauri.conf.json version" 0.1.13-alpha.1 "$(jq -r .version "$copy/apps/$app/src-tauri/tauri.conf.json")"
done
expect "Android versionCode" 101301 "$(jq -r .bundle.android.versionCode "$copy/apps/android/src-tauri/tauri.conf.json")"
expect "the server crate in Cargo.lock" 0.1.13-alpha.1 "$(grep -A1 '^name = "server"$' "$copy/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')"
expect "the desktop app in its Cargo.lock" 0.1.13-alpha.1 "$(grep -A1 '^name = "thevault-desktop"$' "$copy/apps/desktop/src-tauri/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')"
plugin=$(grep -A1 '^name = "tauri-plugin-vault-device"$' "$root/apps/android/src-tauri/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')
expect "the plugin keeps its own version" "$plugin" "$(grep -A1 '^name = "tauri-plugin-vault-device"$' "$copy/apps/android/src-tauri/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')"
serde=$(grep -A1 '^name = "serde"$' "$root/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)
expect "a registry crate keeps its version" "$serde" "$(grep -A1 '^name = "serde"$' "$copy/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)"
changed=$(diff "$root/Cargo.lock" "$copy/Cargo.lock" | grep -c '^>' || true)
expect "Cargo.lock lines changed, one per workspace crate" "$(grep -l '^version\.workspace *= *true' "$root"/crates/*/Cargo.toml | wc -l)" "$changed"

if [ "$failed" -ne 0 ]; then
  echo "release script tests FAILED" >&2
  exit 1
fi
echo "release script tests passed"
