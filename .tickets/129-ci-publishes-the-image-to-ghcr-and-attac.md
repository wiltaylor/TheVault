---
id: '129'
title: CI publishes the image to GHCR and attaches the compose files on a v* tag
blocked_by:
- '127'
- '128'
status: claimed
verified: unknown
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
