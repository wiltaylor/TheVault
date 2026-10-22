---
id: '138'
title: Cache the dev container image between CI jobs and runs, so each job skips the
  ~9 minute rebuild
blocked_by: []
status: done
verified: unmet
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

### 2026-10-22T02:12:27Z — wil

Done on ticket/138 (efa00b3). New .github/workflows/dev-image.yml (workflow_call + workflow_dispatch, packages: write): './dev --print-image' gives ghcr.io/wiltaylor/thevault-dev:<hash> (same 12-char sha256 ./dev tags with); docker manifest inspect; if missing, free disk and './dev --publish-image' (build + push; a failed push is a warning, jobs then build). release-train runs dev-image after version and before desktop/android; android-smoke.yml runs it before smoke. release.yml, android-release.yml and android-smoke-reusable.yml log in to ghcr.io with GITHUB_TOKEN (packages: read) and run './dev true', which pulls and prints 'dev: pulled ... in Ns' or falls back to building. ./dev locally: missing image -> pull -> build (THEVAULT_DEV_PULL=0 skips the pull, THEVAULT_DEV_REGISTRY overrides). Dockerfile: apt Acquire::Retries 5, curl --retry 5 --retry-all-errors --retry-delay 5 to files (no more curl|tar), retry loop (5 tries, 15s) around sdkmanager, npm and Playwright; new hash 0e7728f8d6af built locally in 869s with no retries needed. Verified: actionlint clean, shellcheck clean, ./dev just check passes; registry miss path seen ('not found' -> build); against a local registry:2 on a random port, --publish-image built and pushed, then with the local image removed ./dev pulled it (1s) and ran from it; docker manifest inspect finds ghcr.io/wiltaylor/thevault:latest and misses an absent tag. Not pushed to GHCR locally (docker is logged in as wiltaylor) on purpose: a package a workflow creates is linked to the repo with GITHUB_TOKEN write access, a user-pushed one may start private and unlinked and make CI pushes 403. Unmet until after merge: run Actions > dev-image (workflow_dispatch on main) to create the package, set it public in the package settings, then dispatch android-smoke (release-key false): its dev-image job should report the image is in the registry and the smoke job's 'Pull the dev image' step should log 'dev: pulled ghcr.io/wiltaylor/thevault-dev:0e7728f8d6af in Ns' with no build; then on a machine without the image, 'docker rmi thevault-dev:0e7728f8d6af; ./dev true' should log the pull. Before/after timing on release.yml and android-release.yml shows on the next train.

### 2026-10-22T02:12:27Z — wil

Verified set to unmet.

### 2026-10-22T02:12:27Z — wil

Status set to done.
