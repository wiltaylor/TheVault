---
id: '75'
title: just android-build can hand its Gradle build to another container's daemon
blocked_by: []
status: parked
verified: unknown
park_cause: needs-triage
---

# just android-build can hand its Gradle build to another container's daemon

Every ./dev container shares the /gradle volume (Gradle home, including the daemon registry) and the host network. A Gradle client in one container can therefore connect to an idle daemon started by another container, which does not have the caller's checkout mounted. The build then fails with: Could not set process working directory to '<worktree>/apps/android/src-tauri': could not setcwd() (errno 2). Seen in ticket 71 while several fleet workers ran at once. android-release (ticket 71) avoids it with GRADLE_OPTS=-Dorg.gradle.daemon=false; android-build, which also runs cargo tauri android build, does not. android-test's gradlew call already passes --no-daemon.

## Worklog
