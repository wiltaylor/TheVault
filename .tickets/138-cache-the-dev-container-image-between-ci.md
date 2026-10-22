---
id: '138'
title: Cache the dev container image between CI jobs and runs, so each job skips the
  ~9 minute rebuild
blocked_by: []
status: claimed
verified: unknown
---

# Cache the dev container image between CI jobs and runs, so each job skips the ~9 minute rebuild

Every CI job that uses ./dev rebuilds docker/dev.Dockerfile from scratch (~9 min on ubuntu-24.04, seen in run 39524940666). Options: docker buildx with the GitHub Actions cache backend (type=gha) keyed by the Dockerfile hash that ./dev already uses for its tag, or push the dev image to GHCR as ghcr.io/wiltaylor/thevault-dev:<hash> and pull it. Measure before and after on release.yml, android-release.yml and the smoke job.

## Worklog

### 2026-10-22T01:46:56Z — wil

Status set to todo.

### 2026-10-22T01:46:56Z — wil

Triage 2026-10-22: do now with 122. Approach: build the dev image once per docker/dev.Dockerfile hash and push it to GHCR as ghcr.io/wiltaylor/thevault-dev:<hash> (the hash ./dev already computes); ./dev pulls it when the local image is missing before building; every CI job pulls it; a job that finds no image for the hash builds and pushes it. Also add retries to every download in the Dockerfile: curl --retry 5 --retry-all-errors --retry-delay 5, and a retry loop around sdkmanager installs. Two release runs (alpha.4, 0.1.13) failed on dl.google.com downloads (a corrupt system image zip; curl HTTP/2 stream INTERNAL_ERROR). Acceptance: actionlint; a CI job (dispatch android-smoke) pulls the cached image and skips the build (log shows the pull and its time); a local ./dev on a machine without the image pulls it; ./dev just check passes.

### 2026-10-22T01:47:15Z — wil

Claimed.
