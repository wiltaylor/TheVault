---
id: '64'
title: 'Linux packaging and release: AppImage, deb, rpm, AUR thevault-bin'
plan: thevault-clients
blocked_by:
- '62'
status: todo
verified: unknown
---

# Linux packaging and release: AppImage, deb, rpm, AUR thevault-bin

Read docs/plans/thevault-clients/SPEC.md (the source of truth) and MAP.md. Benchmarks are on hold: never run ./dev just bench.

Implement SPEC distribution for Linux (D11): `just desktop-release` builds AppImage, deb and rpm in the container; an AUR PKGBUILD for `thevault-bin` (in packaging/aur/) installing from GitHub Release artefacts; a GitHub Actions release workflow on tags that builds and uploads them (do not push tags or publish anything - just the workflow file). Document installing on Arch/CachyOS.

Acceptance criteria:
- `./dev just desktop-release` produces the three artefacts; the AppImage launches under Xvfb to the setup page (smoke).
- `makepkg --printsrcinfo` validates the PKGBUILD (run in the container if available).
Decisions: D11.

## Worklog
