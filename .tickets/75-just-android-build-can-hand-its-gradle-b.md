---
id: '75'
title: just android-build can hand its Gradle build to another container's daemon
blocked_by: []
status: done
verified: unmet
---

# just android-build can hand its Gradle build to another container's daemon

Every ./dev container shares the /gradle volume (Gradle home, including the daemon registry) and the host network. A Gradle client in one container can therefore connect to an idle daemon started by another container, which does not have the caller's checkout mounted. The build then fails with: Could not set process working directory to '<worktree>/apps/android/src-tauri': could not setcwd() (errno 2). Seen in ticket 71 while several fleet workers ran at once. android-release (ticket 71) avoids it with GRADLE_OPTS=-Dorg.gradle.daemon=false; android-build, which also runs cargo tauri android build, does not. android-test's gradlew call already passes --no-daemon.

## Worklog

### 2026-10-15T09:39:20Z — wil

Status set to claimed.

### 2026-10-15T09:39:20Z — wil

android-build now passes GRADLE_OPTS=-Dorg.gradle.daemon=false to cargo tauri android build, as android-release does; android-test already used --no-daemon. ./dev just android-build succeeds with it (debug APK built). Not reproduced: the cross-container daemon reuse needs two containers racing on the shared /gradle volume, which I did not set up, so the fix is unproven against the original failure.

### 2026-10-15T09:39:20Z — wil

Verified set to unmet.

### 2026-10-15T09:39:20Z — wil

Status set to done.
