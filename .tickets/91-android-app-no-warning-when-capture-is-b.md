---
id: '91'
title: 'Android app: no warning when capture is blocked by a missing permission, restricted
  settings, battery optimisation or stalled background work'
blocked_by:
- '88'
status: todo
verified: unknown
---

# Android app: no warning when capture is blocked by a missing permission, restricted settings, battery optimisation or stalled background work

On the live phone (Samsung SM-F976B, release 0.1.5) no SMS or call reached the vault after a reinstall, and queued device commands sat pending for over 20 minutes, until the user set the app's battery to Unrestricted, allowed restricted settings, and re-granted SMS and Call log permissions. The app gave no sign of any of it: a missing permission is only logged at info level (DeviceCapture.kt:180-186), and nothing checks battery optimisation or whether background work is being run.

Build, in the app's Settings > Devices > This device and on app start:
1. A "Capture health" panel listing each capture kind (SMS, MMS, calls, contacts, RCS notification access) with its state: working, permission missing, restricted settings blocked (for a sideloaded app on Android 13+ when the permission cannot be requested), or not set up. Each problem has a button that opens the exact system screen to fix it (app permission page, notification-listener settings, app details for restricted settings).
2. Battery: show when the app is battery-optimised (PowerManager.isIgnoringBatteryOptimizations false) with a button that requests the exemption, and on Samsung a line telling the user to keep the app out of Sleeping apps.
3. Background liveness: record when the catch-up worker and the command fetch last ran; when either has not run for 2 hours while the app is paired, show it in the panel.
4. A persistent problem (permission missing, optimised, or background stalled for 2 hours) raises one phone notification that opens the panel, at most once a day per problem.
5. The phone sends its capture health with its settings sync, and the web app's device card shows a warning line when the phone reports a problem.

Acceptance criteria:
- Robolectric tests for each state: permission missing, battery optimised, worker stale.
- The device card in the web app shows the reported warning (web test).
- docs/android-manual-checks.md gains a check for the panel.
- `./dev just check` and the Android unit tests pass.

## Worklog
