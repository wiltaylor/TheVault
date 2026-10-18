---
id: '129'
title: CI publishes the image to GHCR and attaches the compose files on a v* tag
blocked_by:
- '127'
- '128'
status: done
verified: verified
---

# CI publishes the image to GHCR and attaches the compose files on a v* tag

Builds on the image and compose tickets. Publish the image and compose files from CI on a `v*` tag.

Build:
1. In .github/workflows (extend release.yml or add image.yml, whichever keeps the tag flow simplest): a job that runs after the desktop release and Android release jobs have produced the signed AppImage and APK for the tag (download them from the GitHub Release or as workflow artifacts), writes the clients manifest the way `just publish-clients` does, builds docker/Dockerfile for linux/amd64 with that clients context, and pushes ghcr.io/wiltaylor/thevault with tags <version>, <major>.<minor>, and latest, using GITHUB_TOKEN with packages: write.
2. Attach deploy/compose.yaml, deploy/.env.example and deploy/Caddyfile (if separate) to the GitHub Release.
3. The job checks the tag matches the workspace version, as the existing jobs do.
4. OCI labels on the image: source, version, revision, licenses if the repo has one.
5. docs/install.md and docs/android-release.md (or a release doc) describe the release order: tag, then the three jobs, then the image.

Acceptance criteria:
- actionlint (run through ./dev or a container) passes on the workflow files.
- The job's build steps run locally in a script that mirrors them (no push), producing an image with clients from local release artifacts, and `docker run` of it answers /api/clients with the APK.
- A real tag run is not done by this ticket: record in the worklog the exact command to cut the next release and what to check on GHCR afterwards, for the user to approve.
- ./dev just check passes.

## Worklog

### 2026-10-18T09:37:35Z — /work

Claimed.

### 2026-10-18T09:56:06Z — wil

Verified set to verified.

### 2026-10-18T09:56:06Z — wil

Done on ticket/129 (90409a9). New .github/workflows/image.yml runs on workflow_run of release and android-release (plus workflow_dispatch with a tag): the run that finds the APK, AppImage and both CLI builds on the release builds, checks and pushes ghcr.io/wiltaylor/thevault <version>, <major>.<minor>, latest via docker/release-image.sh --push, then attaches compose.yaml, Caddyfile and env.example (GitHub renames leading-dot assets to default.*, so .env.example is uploaded as env.example; docs/install.md updated). release.yml now also runs just cli-release and attaches vault-<v>-linux-{x86_64,aarch64} + SHA256SUMS-cli. stage-clients.sh now stages kind binary. Local mirror: just image-release <assets-dir> (no push). Proven: actionlint (rhysd/actionlint) 0 errors on all three workflows; docker/release-image.sh on local 0.1.11 artifacts built the image and its own docker run served the APK, AppImage (as TheVault_0.1.11_x86_64.AppImage) and both CLI builds with matching sha256; wrong tag, missing asset and bad checksum each exit 1 before building; docker/test-compose.sh all checks passed; ./dev just check exit 0. NOT done (needs user approval): a real tag run. Release command: merge ticket/129 into main and push main FIRST (workflow_run only fires for workflows on the default branch), set version 0.1.12 in Cargo.toml [workspace.package], apps/desktop/src-tauri/tauri.conf.json and apps/android/src-tauri/tauri.conf.json, commit, push, then: git tag v0.1.12 && git push origin v0.1.12. Afterwards check: Actions shows release, android-release, then image (one image run publishes, the other says it stops); the release lists the APK, AppImage, deb, rpm, AUR tarball, vault-0.1.12-linux-x86_64/aarch64, SHA256SUMS, SHA256SUMS-android, SHA256SUMS-cli, compose.yaml, Caddyfile, env.example; github.com/wiltaylor?tab=packages shows thevault private with tags 0.1.12, 0.1 and latest linked to the repo; docker pull ghcr.io/wiltaylor/thevault:0.1.12 then docker image inspect shows the org.opencontainers.image.* labels; curl -fLO .../releases/latest/download/env.example works. Filed: AppImage has no updater signature in CI; CI APK skips the smoke gate.

### 2026-10-18T09:56:06Z — wil

Status set to done.
