---
id: '64'
title: 'Linux packaging and release: AppImage, deb, rpm, AUR thevault-bin'
plan: thevault-clients
blocked_by:
- '62'
status: done
verified: verified
---

# Linux packaging and release: AppImage, deb, rpm, AUR thevault-bin

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC distribution for Linux (D11): `just desktop-release` builds AppImage, deb and rpm in the container; an AUR PKGBUILD for `thevault-bin` (in packaging/aur/) installing from GitHub Release artefacts; a GitHub Actions release workflow on tags that builds and uploads them (do not push tags or publish anything - just the workflow file). Document installing on Arch/CachyOS.

Acceptance criteria:
- `./dev just desktop-release` produces the three artefacts; the AppImage launches under Xvfb to the setup page (smoke).
- `makepkg --printsrcinfo` validates the PKGBUILD (run in the container if available).
Decisions: D11.

## Worklog

### 2026-10-15T08:14:45Z — /work

Claimed.

### 2026-10-15T08:28:06Z — wil

Verified set to verified.

### 2026-10-15T08:28:07Z — wil

Status set to done.

### 2026-10-15T08:28:07Z — wil

Committed d4ce6a5 on ticket/thevault-clients-64, branched from ticket/thevault-clients-62 (66b1a57) because 62 was done but not on main: merge 62 first. ./dev just desktop-release-test built TheVault_0.1.0_amd64.AppImage, TheVault_0.1.0_amd64.deb and TheVault-0.1.0-1.x86_64.rpm into target/desktop-release and the smoke (apps/desktop/e2e/appimage.smoke.mjs, AppImage via tauri-driver with APPIMAGE_EXTRACT_AND_RUN=1) passed 1/1; pointed at a missing AppImage it fails. ./dev just aur-check passes (makepkg --printsrcinfo matches packaging/aur/.SRCINFO). The rendered PKGBUILD also built with makepkg -d against the local deb, installing /usr/bin/thevault, the desktop entry and icons (Tauri's 256x256@2 icon dir moved to 256x256). Dev image gains makepkg + pacman-package-manager (Debian makepkg needs the pacman binary) + libarchive-tools. Assumed the GitHub repo is github.com/wiltaylor/thevault (no remote configured); change url= in packaging/aur/PKGBUILD if not. The release workflow was not run (no tag pushed, by design); it builds the whole dev image on the runner, so disk space may need watching. Set bundle.category=Productivity in tauri.conf.json (desktop entry had empty Categories). ./dev just check exits 0.

### 2026-10-15T08:46:49Z — wil

Rebased onto main (abcec58) as 30a4ca0; ./dev just check, just desktop-release-test (AppImage smoke 1/1) and just aur-check pass on it; main fast-forwarded to 30a4ca0.
