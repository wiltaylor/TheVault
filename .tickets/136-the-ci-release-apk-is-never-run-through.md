---
id: '136'
title: The CI release APK is never run through android-release-smoke (the gate just
  publish-clients enforces with SMOKE_PASSED), so image.yml bakes in an APK only the
  manual pre-tag smoke run vouches for; decide whether CI should record or require
  the smoke result
blocked_by: []
status: todo
verified: unknown
---

# The CI release APK is never run through android-release-smoke (the gate just publish-clients enforces with SMOKE_PASSED), so image.yml bakes in an APK only the manual pre-tag smoke run vouches for; decide whether CI should record or require the smoke result

## Worklog

### 2026-10-21T01:32:53Z — wil

Status set to todo.

### 2026-10-21T01:32:53Z — wil

Spike (2026-10-21, run 39524940666, .github/workflows/android-smoke.yml): KVM on ubuntu-24.04 works with the udev rule from GitHub's 2024-04-02 changelog; ./dev just android-release-smoke passed first try in 15 min (APK build ~7, server+cli ~2, emulator boot ~1, checks ~6), job 27 min incl. 9 min dev image. User decided to finish it: in android-release.yml add a smoke job running in parallel with the build (KVM step, release keystore from secrets so it smoke-tests a release-signed x86_64 build, ./dev just android-release-smoke, upload .scratch/emulator-release as an artifact always); the step that attaches the APK to the release runs only when the smoke job passed, so a failing smoke publishes no APK (and the image job, which needs the APK, publishes nothing). Keep one source of truth for the smoke steps (a reusable workflow called by android-release.yml and by the hand-started android-smoke.yml). Update docs/android-release.md: the local smoke run before tagging is no longer required; publish-clients' local SMOKE_PASSED rule still applies. Acceptance: actionlint passes; the hand-started android-smoke workflow still passes through the reusable workflow; the gate is shown by the workflow structure (needs/if) and, if the user approves a test tag, by a real run; ./dev just check passes.
