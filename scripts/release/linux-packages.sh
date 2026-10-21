#!/usr/bin/env bash
# Give a pre-release's deb and rpm the version their package managers sort before the release
# (ticket 139, docs/releasing.md). Tauri writes the app's semver into the deb, which dpkg reads
# as upstream 0.1.13 with revision alpha.1 and sorts after 0.1.13, and cannot write it into an
# rpm, whose version holds no -, so `just desktop-release` bundles the rpm as 0.1.13. This
# rebuilds each package in <dir> with scripts/release/package-version.sh's version,
# 0.1.13~alpha.1, and names it by the semver, TheVault_0.1.13-alpha.1_amd64.deb and
# TheVault-0.1.13-alpha.1-1.x86_64.rpm, as every file of the release is. A release, 0.1.13, is
# left as it is.
#
#     scripts/release/linux-packages.sh <dir> <version>
#
# Needs dpkg-deb, rpm, rpmbuild and bsdtar: run it through ./dev.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
dir=${1:-}
version=${2:-}
if [ -z "$dir" ] || [ ! -d "$dir" ] || [ -z "$version" ]; then
  echo "usage: $0 <dir> <version>" >&2
  exit 2
fi
dir=$(cd "$dir" && pwd)
if [[ $version != *-* ]]; then
  echo "$version is a release: the deb and rpm keep it"
  exit 0
fi
deb_version=$("$here/package-version.sh" deb "$version")
rpm_version=$("$here/package-version.sh" rpm "$version")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
shopt -s nullglob

for deb in "$dir"/*.deb; do
  rm -rf "$work/deb" && mkdir -p "$work/deb"
  dpkg-deb -R "$deb" "$work/deb/root"
  sed -i "s/^Version: .*/Version: $deb_version/" "$work/deb/root/DEBIAN/control"
  # gzip, as Tauri writes it: the AUR package reads data.tar.gz.
  dpkg-deb -Zgzip --root-owner-group -b "$work/deb/root" "$work/deb/out.deb" > /dev/null
  mv "$work/deb/out.deb" "$deb"
  echo "$deb: Version $(dpkg-deb -f "$deb" Version)"
done

for rpm in "$dir"/*.rpm; do
  top="$work/rpm"
  rm -rf "$top" && mkdir -p "$top/root" "$top/SPECS"
  q() { rpm -qp --nosignature --qf "$1" "$rpm"; }
  if [ -n "$(rpm -qp --nosignature --scripts "$rpm")" ]; then
    echo "$rpm has install scripts, which this rebuild does not carry over" >&2
    exit 1
  fi
  # bsdtar reads the payload of an rpm Tauri wrote; rpm2cpio extracts it but then exits 1.
  bsdtar -xf "$rpm" -C "$top/root"
  name=$(q '%{NAME}') release=$(q '%{RELEASE}') arch=$(q '%{ARCH}') epoch=$(q '%{EPOCH}')
  {
    echo "Name: $name"
    if [ "$epoch" != "(none)" ] && [ "$epoch" != 0 ]; then echo "Epoch: $epoch"; fi
    echo "Version: $rpm_version"
    echo "Release: $release"
    echo "Summary: $(q '%{SUMMARY}')"
    echo "License: $(q '%{LICENSE}')"
    url=$(q '%{URL}')
    if [ "$url" != "(none)" ]; then echo "URL: $url"; fi
    # The dependencies Tauri wrote, and nothing rpmbuild would find by itself.
    echo "AutoReqProv: no"
    rpm -qp --nosignature --requires "$rpm" | grep -v '^rpmlib(' | sed 's/^/Requires: /' || true
    rpm -qp --nosignature --provides "$rpm" | grep -v "^$name\\( \\|$\\)" | sed 's/^/Provides: /' || true
    rpm -qp --nosignature --conflicts "$rpm" | sed 's/^/Conflicts: /' || true
    rpm -qp --nosignature --obsoletes "$rpm" | sed 's/^/Obsoletes: /' || true
    echo '%global debug_package %{nil}'
    echo '%global __os_install_post %{nil}'
    echo '%global _build_id_links none'
    echo '%description'
    q '%{DESCRIPTION}'
    echo
    echo '%install'
    echo "cp -a '$top/root/.' %{buildroot}/"
    echo '%files'
    echo '%defattr(-,root,root,-)'
    q '[%{FILEMODES:perms}\t%{FILENAMES}\n]' | while IFS=$'\t' read -r mode path; do
      case "$mode" in
        d*) echo "%dir \"$path\"" ;;
        *) echo "\"$path\"" ;;
      esac
    done
  } > "$top/SPECS/package.spec"
  rpmbuild -bb --quiet --target "$arch" --define "_topdir $top" "$top/SPECS/package.spec" > /dev/null
  built=$(find "$top/RPMS" -name '*.rpm' | head -n 1)
  test -n "$built" || { echo "rpmbuild wrote no rpm for $rpm" >&2; exit 1; }
  # The file name names the semver: TheVault-0.1.13-1.x86_64.rpm becomes TheVault-0.1.13-alpha.1-1.x86_64.rpm.
  named="$(dirname "$rpm")/$(basename "$rpm" | sed "s/-$(q '%{VERSION}' | sed 's/[.]/\\./g')-/-$version-/")"
  rm -f "$rpm"
  mv "$built" "$named"
  rpm=$named
  echo "$rpm: Version $(q '%{VERSION}-%{RELEASE}')"
done
